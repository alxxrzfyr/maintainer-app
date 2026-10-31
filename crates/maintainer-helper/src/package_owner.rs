//! Short-lived owner for one retained native APT transaction.
//!
//! Only the fixed installed backend executes package operations. Receipts are
//! backend-owned; losing the owner never replays a transaction or kills APT.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::api::actions;
use crate::authorize::{AuthorizePolicy, CallerCredentials};
use maintainer_core::package_transaction::{MAX_REQUESTS, PackagePhase, PackageReceipt};

type Error = zbus::fdo::Error;
type Result<T> = std::result::Result<T, Error>;
const MAX_RECEIPT_BYTES: u64 = maintainer_core::package_transaction::MAX_RECEIPT_BYTES as u64;
pub const BUS_NAME: &str = "io.github.maintainerapp.PackageOwner";
pub const OBJECT_PATH: &str = "/io/github/maintainerapp/PackageOwner";

struct Running {
    child: Child,
    input: Option<ChildStdin>,
    start_ticks: u64,
    uid: u32,
    bus: String,
    request: String,
}

struct State {
    running: HashMap<String, Running>,
    last_request: Instant,
    startup_failures: HashMap<String, String>,
}

struct Owner {
    policy: Arc<dyn AuthorizePolicy>,
    state: Mutex<State>,
    receipts: PathBuf,
    backend: PathBuf,
}

/// Fixed production configuration; test fixtures replace paths only in tests.
#[derive(Clone)]
pub struct PackageOwner {
    owner: Arc<Owner>,
    requests: Arc<tokio::sync::Semaphore>,
    runtime: Option<tokio::runtime::Handle>,
}

fn failed(reason: impl ToString) -> Error {
    Error::Failed(reason.to_string())
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn validate_id(id: &str) -> Result<()> {
    if maintainer_core::package_transaction::valid_transaction_id(id) {
        Ok(())
    } else {
        Err(Error::InvalidArgs(
            "transaction ID must be 32 lowercase hexadecimal characters".into(),
        ))
    }
}

fn initial_request(intent: &str, packages: &[String]) -> Result<String> {
    if packages.len() > MAX_REQUESTS {
        return Err(Error::InvalidArgs("at most 256 selected packages".into()));
    }
    match intent {
        "full" if packages.is_empty() => Ok("FULL\n".into()),
        "selected" => {
            let mut args = vec!["selected".into()];
            args.extend_from_slice(packages);
            crate::validate::parse_install_updates_cli(&args)
                .map_err(|error| Error::InvalidArgs(error.to_string()))?;
            let mut names = std::collections::HashSet::new();
            for spec in packages {
                let (qualified, version) = spec
                    .split_once('=')
                    .ok_or_else(|| Error::InvalidArgs("invalid package specification".into()))?;
                let mut components = qualified.split(':');
                let name = components.next().unwrap_or_default();
                let architecture = components.next();
                let name_ok = !name.is_empty()
                    && name.len() <= 128
                    && name.as_bytes()[0].is_ascii_alphanumeric()
                    && name.bytes().all(|b| {
                        b.is_ascii_lowercase()
                            || b.is_ascii_digit()
                            || matches!(b, b'+' | b'.' | b'-')
                    });
                let arch_ok = architecture.is_none_or(|arch| {
                    !arch.is_empty()
                        && arch.len() <= 32
                        && arch.as_bytes()[0].is_ascii_alphanumeric()
                        && arch
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                });
                if !name_ok
                    || !arch_ok
                    || components.next().is_some()
                    || version.len() > 256
                    || !version
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphanumeric)
                    || !names.insert(qualified)
                {
                    return Err(Error::InvalidArgs(
                        "invalid or duplicate native package specification".into(),
                    ));
                }
            }
            Ok(format!("SELECTED\t{}\n", packages.join("\t")))
        }
        "kernel" if packages.len() == 1 => {
            let name = &packages[0];
            crate::validate::validate_remove_kernel(&crate::api::RemoveKernelRequest {
                package_name: name.clone(),
            })
            .map_err(|error| Error::InvalidArgs(error.to_string()))?;
            Ok(format!("KERNEL\t{name}\n"))
        }
        _ => Err(Error::InvalidArgs(
            "intent must be full, selected or kernel with matching packages".into(),
        )),
    }
}

impl PackageOwner {
    pub fn new(policy: Arc<dyn AuthorizePolicy>) -> Self {
        Self::configured(
            policy,
            PathBuf::from("/var/lib/maintainer/transactions"),
            PathBuf::from("/usr/libexec/maintainer-apt-backend.py"),
        )
    }

    fn configured(policy: Arc<dyn AuthorizePolicy>, receipts: PathBuf, backend: PathBuf) -> Self {
        Self {
            owner: Arc::new(Owner {
                policy,
                state: Mutex::new(State {
                    running: HashMap::new(),
                    last_request: Instant::now(),
                    startup_failures: HashMap::new(),
                }),
                receipts,
                backend,
            }),
            requests: Arc::new(tokio::sync::Semaphore::new(1)),
            runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    async fn dispatch<T: Send + 'static>(
        &self,
        operation: impl FnOnce(Arc<Owner>) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let permit = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::LimitsExceeded("package owner is busy".into()))?;
        let owner = self.owner.clone();
        self.runtime
            .as_ref()
            .ok_or_else(|| failed("owner runtime unavailable"))?
            .spawn_blocking(move || {
                let _permit = permit;
                operation(owner)
            })
            .await
            .map_err(failed)?
    }
}

impl Owner {
    fn caller(&self, sender: &str) -> Result<CallerCredentials> {
        self.policy
            .resolve_caller(sender)
            .map_err(|_| Error::AccessDenied("caller identity unavailable".into()))
    }

    fn authorize(&self, caller: &CallerCredentials, intent: &str) -> Result<()> {
        let action = if intent == "kernel" {
            actions::REMOVE_KERNEL
        } else {
            actions::INSTALL_UPDATES
        };
        self.policy
            .check_authorization(caller, action)
            .map_err(|error| Error::AccessDenied(error.to_string()))
    }

    fn state(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| failed("owner state unavailable"))?;
        state.last_request = Instant::now();
        let mut finished = Vec::new();
        for (id, running) in &mut state.running {
            if running.child.try_wait().map_err(failed)?.is_some() {
                finished.push(id.clone());
            }
        }
        for id in finished {
            if let Some(running) = state.running.remove(&id) {
                self.record_startup_failure(&id, &running, &mut state)?;
            }
        }
        Ok(state)
    }

    fn record_startup_failure(&self, id: &str, running: &Running, state: &mut State) -> Result<()> {
        if self.receipts.join(id).symlink_metadata().is_ok() {
            return Ok(());
        }
        let mut fields = running.request.trim_end().split('\t');
        let intent = fields
            .next()
            .ok_or_else(|| failed("missing backend intent"))?
            .to_ascii_lowercase();
        let mut requests = String::new();
        for package in fields {
            requests.push_str("REQUEST\t");
            requests.push_str(package);
            requests.push('\n');
        }
        let receipt = format!(
            "VERSION\t1\nID\t{id}\nUID\t{}\nPID\t{}\nSTART\t{}\nSTATE\tfailed\nINTENT\t{intent}\n{requests}PLAN\t-\t0\nERROR\tbackend_startup_failed\nEND\n",
            running.uid,
            running.child.id(),
            running.start_ticks
        );
        // The child is reaped and cannot publish a receipt now. Never replace
        // any existing backend record, including malformed recovery evidence.
        let persist = || -> std::io::Result<()> {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&self.receipts)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(self.receipts.join(id))?;
            file.write_all(receipt.as_bytes())?;
            file.sync_all()
        };
        if persist().is_err() {
            // Bound admission rather than forgetting accepted IDs and replaying.
            if state.startup_failures.len() >= 32 {
                return Err(failed("startup failure storage unavailable"));
            }
            state.startup_failures.insert(id.into(), receipt);
        }
        Ok(())
    }

    fn receipt(&self, id: &str, uid: u32) -> Result<Option<String>> {
        validate_id(id)?;
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.receipts.join(id))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(failed(error)),
        };
        if !file.metadata().map_err(failed)?.is_file() {
            return Err(failed("invalid receipt"));
        }
        let mut receipt = String::new();
        (&mut file)
            .take(MAX_RECEIPT_BYTES + 1)
            .read_to_string(&mut receipt)
            .map_err(failed)?;
        let parsed = PackageReceipt::parse(&receipt)
            .map_err(|_| failed("invalid or oversized backend receipt"))?;
        if parsed.id != id {
            return Err(failed("receipt transaction mismatch"));
        }
        if parsed.uid != uid {
            return Err(Error::AccessDenied(
                "transaction belongs to another user".into(),
            ));
        }
        Ok(Some(receipt))
    }

    fn begin(&self, sender: &str, intent: &str, packages: &[String], id: &str) -> Result<String> {
        validate_id(id)?;
        // Canonical order binds retries to the same request, independent of UI order.
        if packages.len() > MAX_REQUESTS {
            return Err(Error::InvalidArgs("at most 256 selected packages".into()));
        }
        let mut packages = packages.to_vec();
        packages.sort();
        if packages.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::InvalidArgs("duplicate package specification".into()));
        }
        let initial = initial_request(intent, &packages)?;
        let caller = self.caller(sender)?;
        let mut state = self.state()?;
        if let Some(receipt) = state.startup_failures.get(id) {
            let parsed = PackageReceipt::parse(receipt).map_err(failed)?;
            if parsed.uid != caller.uid {
                return Err(Error::AccessDenied(
                    "transaction belongs to another user".into(),
                ));
            }
            if parsed.intent != intent || parsed.requests != packages {
                return Err(Error::InvalidArgs(
                    "request ID already identifies a different intent".into(),
                ));
            }
            return Ok(id.into());
        }
        if state.startup_failures.len() >= 32 {
            return Err(Error::LimitsExceeded(
                "startup failure storage unavailable".into(),
            ));
        }
        if let Some(running) = state.running.get(id) {
            if running.uid != caller.uid {
                return Err(Error::AccessDenied(
                    "transaction belongs to another user".into(),
                ));
            }
            if running.request != initial {
                return Err(Error::InvalidArgs(
                    "request ID already identifies a different intent".into(),
                ));
            }
            return Ok(id.into());
        }
        if let Some(receipt) = self.receipt(id, caller.uid)? {
            let parsed = PackageReceipt::parse(&receipt).map_err(failed)?;
            if parsed.intent != intent || parsed.requests != packages {
                return Err(Error::InvalidArgs(
                    "request ID already identifies a different intent".into(),
                ));
            }
            return Ok(id.into());
        }
        if !state.running.is_empty() {
            return Err(Error::LimitsExceeded(
                "another transaction is active".into(),
            ));
        }
        self.authorize(&caller, intent)?;
        let running = self.spawn_backend(sender, caller.uid, id, initial)?;
        state.running.insert(id.into(), running);
        Ok(id.into())
    }

    fn spawn_backend(&self, sender: &str, uid: u32, id: &str, request: String) -> Result<Running> {
        let mut child = Command::new("/usr/bin/python3")
            .arg("-I")
            .arg(&self.backend)
            .arg(uid.to_string())
            .arg(id)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LANG", "C.UTF-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(failed)?;
        let start_ticks = process_start(child.id()).unwrap_or(1);
        let mut input = child.stdin.take();
        if let Some(pipe) = input.as_mut() {
            if pipe.write_all(request.as_bytes()).is_err() {
                input = None;
            }
        }
        Ok(Running {
            child,
            input,
            start_ticks,
            uid,
            bus: sender.into(),
            request,
        })
    }

    fn reconcile(&self, sender: &str, id: &str) -> Result<()> {
        validate_id(id)?;
        let caller = self.caller(sender)?;
        let mut state = self.state()?;
        if !state.running.is_empty() {
            return Err(Error::LimitsExceeded(
                "another transaction is active".into(),
            ));
        }
        let receipt = self
            .receipt(id, caller.uid)?
            .ok_or_else(|| Error::UnknownObject("transaction not found".into()))?;
        let parsed = PackageReceipt::parse(&receipt).map_err(failed)?;
        if process_start(parsed.backend_pid) == Some(parsed.backend_start_ticks) {
            return Err(Error::LimitsExceeded("backend is still active".into()));
        }
        if parsed.phase.is_terminal() && parsed.phase != PackagePhase::RecoveryRequired {
            return Err(Error::InvalidArgs(
                "transaction does not require recovery audit".into(),
            ));
        }
        self.authorize(&caller, &parsed.intent)?;
        let running = self.spawn_backend(sender, caller.uid, id, "RECONCILE\n".into())?;
        state.running.insert(id.into(), running);
        Ok(())
    }

    fn status(&self, sender: &str, id: &str) -> Result<(String, bool)> {
        validate_id(id)?;
        let caller = self.caller(sender)?;
        let state = self.state()?;
        let mut receipt = match self.receipt(id, caller.uid)? {
            Some(receipt) => receipt,
            None => {
                if let Some(receipt) = state.startup_failures.get(id) {
                    let parsed = PackageReceipt::parse(receipt).map_err(failed)?;
                    if parsed.uid != caller.uid {
                        return Err(Error::AccessDenied(
                            "transaction belongs to another user".into(),
                        ));
                    }
                    receipt.clone()
                } else if state
                    .running
                    .get(id)
                    .is_some_and(|running| running.uid == caller.uid)
                {
                    return Err(failed("backend receipt pending"));
                } else {
                    return Err(Error::UnknownObject("transaction not found".into()));
                }
            }
        };
        let parsed = PackageReceipt::parse(&receipt).map_err(failed)?;
        if !parsed.phase.is_terminal()
            && process_start(parsed.backend_pid) != Some(parsed.backend_start_ticks)
        {
            let phase_line = receipt
                .lines()
                .find(|line| line.starts_with("STATE\t"))
                .ok_or_else(|| failed("missing transaction state"))?;
            receipt = receipt.replacen(&format!("{phase_line}\n"), "STATE\trecovery_required\n", 1);
        }
        let control = state
            .running
            .get(id)
            .is_some_and(|running| running.uid == caller.uid && running.input.is_some())
            && parsed.phase.can_cancel();
        Ok((receipt, control))
    }

    fn recent(&self, sender: &str) -> Result<Vec<String>> {
        let caller = self.caller(sender)?;
        let state = self.state()?;
        let entries: Vec<_> = match std::fs::read_dir(&self.receipts) {
            Ok(entries) => entries.take(1025).collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(failed(error)),
        };
        let mut candidates = Vec::new();
        for (index, entry) in entries.into_iter().enumerate() {
            if index >= 1024 {
                return Err(Error::LimitsExceeded(
                    "too many transaction receipts".into(),
                ));
            }
            let entry = entry.map_err(failed)?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if !valid_hex(&id, 32) {
                continue;
            }
            let metadata = entry.metadata().map_err(failed)?;
            if !metadata.is_file() {
                continue;
            }
            match self.receipt(&id, caller.uid) {
                Ok(Some(_)) => candidates.push((metadata.modified().map_err(failed)?, id)),
                Err(Error::AccessDenied(_)) | Ok(None) => {}
                Err(error) => return Err(error),
            }
        }
        for (id, receipt) in &state.startup_failures {
            let parsed = PackageReceipt::parse(receipt).map_err(failed)?;
            if parsed.uid == caller.uid && !candidates.iter().any(|(_, found)| found == id) {
                candidates.push((std::time::SystemTime::now(), id.clone()));
            }
        }
        candidates.sort_by(|left, right| right.cmp(left));
        Ok(candidates.into_iter().take(32).map(|(_, id)| id).collect())
    }

    fn control(&self, sender: &str, id: &str, fingerprint: Option<&str>) -> Result<bool> {
        validate_id(id)?;
        if fingerprint.is_some_and(|value| !valid_hex(value, 64)) {
            return Err(Error::InvalidArgs("invalid plan fingerprint".into()));
        }
        let caller = self.caller(sender)?;
        let mut state = self.state()?;
        let receipt = self
            .receipt(id, caller.uid)?
            .ok_or_else(|| failed("backend receipt pending"))?;
        let parsed = PackageReceipt::parse(&receipt).map_err(failed)?;
        if (fingerprint.is_some() && parsed.phase != PackagePhase::Review)
            || !parsed.phase.can_cancel()
        {
            return Ok(false);
        }
        if fingerprint.is_some_and(|value| parsed.fingerprint.as_deref() != Some(value)) {
            return Err(Error::InvalidArgs(
                "reviewed plan fingerprint changed".into(),
            ));
        }
        let running = state.running.get_mut(id).ok_or_else(|| {
            failed("transaction control unavailable after owner restart; inspect recovery status")
        })?;
        if running.uid != caller.uid {
            return Err(Error::AccessDenied(
                "transaction belongs to another user".into(),
            ));
        }
        if let Some(fingerprint) = fingerprint {
            if running.bus != sender {
                self.authorize(&caller, &parsed.intent)?;
            }
            running
                .input
                .as_mut()
                .ok_or_else(|| failed("backend control unavailable"))?
                .write_all(format!("APPROVE\t{fingerprint}\n").as_bytes())
                .map_err(failed)?;
            running.bus = sender.into();
        } else {
            running
                .input
                .as_mut()
                .ok_or_else(|| failed("backend control unavailable"))?
                .write_all(b"CANCEL\n")
                .map_err(failed)?;
        }
        // This acknowledges delivery only; Status reports backend cancellation.
        Ok(true)
    }
}

fn process_start(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

fn sender(header: &zbus::message::Header<'_>) -> Result<String> {
    header
        .sender()
        .map(ToString::to_string)
        .ok_or_else(|| Error::AccessDenied("missing sender".into()))
}

#[zbus::interface(name = "io.github.maintainerapp.PackageOwner")]
impl PackageOwner {
    async fn begin(
        &self,
        intent: String,
        packages: Vec<String>,
        request_id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<String> {
        let sender = sender(&header)?;
        self.dispatch(move |owner| owner.begin(&sender, &intent, &packages, &request_id))
            .await
    }
    async fn status(
        &self,
        id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<(String, bool)> {
        let sender = sender(&header)?;
        self.dispatch(move |owner| owner.status(&sender, &id)).await
    }
    async fn recent(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<Vec<String>> {
        let sender = sender(&header)?;
        self.dispatch(move |owner| owner.recent(&sender)).await
    }
    async fn reconcile(
        &self,
        id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<()> {
        let sender = sender(&header)?;
        self.dispatch(move |owner| owner.reconcile(&sender, &id))
            .await
    }
    async fn approve(
        &self,
        id: String,
        fingerprint: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<()> {
        let sender = sender(&header)?;
        self.dispatch(move |owner| {
            if owner.control(&sender, &id, Some(&fingerprint))? {
                Ok(())
            } else {
                Err(failed("transaction is not awaiting approval"))
            }
        })
        .await
    }
    async fn cancel(
        &self,
        id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<bool> {
        let sender = sender(&header)?;
        self.dispatch(move |owner| owner.control(&sender, &id, None))
            .await
    }
}

/// Run only this dedicated service, without widening the maintenance helper.
pub async fn run(
    policy: Arc<dyn AuthorizePolicy>,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let interface = PackageOwner::new(policy);
    let owner = interface.owner.clone();
    let _connection = zbus::connection::Builder::system()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, interface)?
        .build()
        .await?;
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let mut state = owner.state.lock().map_err(|_| "owner state unavailable")?;
        let mut finished = Vec::new();
        for (id, running) in &mut state.running {
            if running.child.try_wait()?.is_some() {
                finished.push(id.clone());
            }
        }
        for id in finished {
            if let Some(running) = state.running.remove(&id) {
                owner.record_startup_failure(&id, &running, &mut state)?;
            }
        }
        if state.running.is_empty() && state.last_request.elapsed() >= Duration::from_secs(60) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const ID: &str = "1234567890abcdef1234567890abcdef";
    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    struct Policy {
        deny: AtomicBool,
        deny_kernel: AtomicBool,
        prompts: AtomicUsize,
    }
    impl AuthorizePolicy for Policy {
        fn resolve_caller(
            &self,
            sender: &str,
        ) -> std::result::Result<CallerCredentials, crate::api::HelperError> {
            Ok(CallerCredentials {
                uid: if sender == ":other-user" { 1001 } else { 1000 },
                pid: 123,
                bus_name: sender.into(),
            })
        }
        fn check_authorization(
            &self,
            _: &CallerCredentials,
            action: &str,
        ) -> std::result::Result<(), crate::api::HelperError> {
            assert!(matches!(
                action,
                actions::INSTALL_UPDATES | actions::REMOVE_KERNEL
            ));
            self.prompts.fetch_add(1, Ordering::SeqCst);
            if self.deny.load(Ordering::SeqCst)
                || (action == actions::REMOVE_KERNEL && self.deny_kernel.load(Ordering::SeqCst))
            {
                Err(crate::api::HelperError::AuthorizationDenied {
                    action: action.into(),
                })
            } else {
                Ok(())
            }
        }
    }
    struct Fixture {
        root: PathBuf,
        owner: PackageOwner,
        policy: Arc<Policy>,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "maintainer-owner-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir(&root).unwrap();
            let backend = root.join("backend.py");
            std::fs::write(&backend, format!(r#"import os, pathlib, sys
root = pathlib.Path({root:?})
uid, ident = sys.argv[1:]
request = sys.stdin.readline().strip().split('\t')
phase = 'review'
def save():
    start = pathlib.Path('/proc/self/stat').read_text().rsplit(')', 1)[1].split()[19]
    text = f'VERSION\t1\nID\t{{ident}}\nUID\t{{uid}}\nPID\t{{os.getpid()}}\nSTART\t{{start}}\nSTATE\t{{phase}}\nINTENT\t{{request[0].lower()}}\n'
    text += ''.join('REQUEST\t' + package + '\n' for package in request[1:])
    text += 'PLAN\t{HASH}\t0\nEND\n'
    temporary = root / (ident + '.tmp')
    temporary.write_text(text)
    temporary.replace(root / ident)
save()
command = sys.stdin.readline().strip()
phase = 'succeeded' if command == 'APPROVE\t{HASH}' else 'cancelled'
save()
"#, root=root.to_string_lossy())).unwrap();
            let policy = Arc::new(Policy {
                deny: AtomicBool::new(false),
                deny_kernel: AtomicBool::new(false),
                prompts: AtomicUsize::new(0),
            });
            let owner = PackageOwner::configured(policy.clone(), root.clone(), backend);
            Self {
                root,
                owner,
                policy,
            }
        }
        fn wait_status(&self, phase: &str) -> (String, bool) {
            for _ in 0..200 {
                if let Ok(status) = self.owner.owner.status(":original", ID) {
                    if status.0.contains(&format!("STATE\t{phase}\n")) {
                        return status;
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("backend did not reach {phase}")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Ok(mut state) = self.owner.owner.state.lock() {
                for running in state.running.values_mut() {
                    if let Some(input) = running.input.as_mut() {
                        let _result = input.write_all(b"CANCEL\n");
                    }
                }
                for running in state.running.values_mut() {
                    let _result = running.child.wait();
                }
            }
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn validation_precedes_authorization_and_spawn() {
        let fixture = Fixture::new();
        for (intent, packages, id) in [
            ("full", vec![], "../bad"),
            ("shell", vec![], ID),
            ("selected", vec!["--option=bad".into()], ID),
            ("selected", vec!["p=1".into(); 257], ID),
        ] {
            assert!(
                fixture
                    .owner
                    .owner
                    .begin(":original", intent, &packages, id)
                    .is_err()
            );
        }
        assert_eq!(fixture.policy.prompts.load(Ordering::SeqCst), 0);
        assert!(fixture.owner.owner.state().unwrap().running.is_empty());
    }

    #[test]
    fn missing_or_early_exiting_backend_becomes_durable_failure_without_replay() {
        for mode in ["missing", "exit", "closed-input"] {
            let fixture = Fixture::new();
            let backend = fixture.root.join("backend.py");
            if mode == "missing" {
                std::fs::remove_file(&backend).unwrap();
            } else {
                std::fs::write(&backend, "import os\nos.close(0)\nraise SystemExit(7)\n").unwrap();
            }
            let (intent, packages) = if mode == "closed-input" {
                (
                    "selected",
                    (0..256)
                        .map(|index| format!("package-{index}={}", "1".repeat(256)))
                        .collect::<Vec<_>>(),
                )
            } else {
                ("full", Vec::new())
            };
            assert_eq!(
                fixture
                    .owner
                    .owner
                    .begin(":original", intent, &packages, ID)
                    .unwrap(),
                ID
            );
            let (receipt, control) = fixture.wait_status("failed");
            assert!(receipt.contains("ERROR\tbackend_startup_failed\n"));
            assert!(!control);
            assert!(fixture.owner.owner.state().unwrap().running.is_empty());
            assert_eq!(fixture.owner.owner.recent(":original").unwrap(), vec![ID]);
            assert!(
                fixture
                    .owner
                    .owner
                    .recent(":other-user")
                    .unwrap()
                    .is_empty()
            );
            let restarted =
                PackageOwner::configured(fixture.policy.clone(), fixture.root.clone(), backend);
            assert_eq!(
                restarted
                    .owner
                    .begin(":original", intent, &packages, ID)
                    .unwrap(),
                ID
            );
            assert!(
                restarted
                    .owner
                    .status(":original", ID)
                    .unwrap()
                    .0
                    .contains("STATE\tfailed\n")
            );
            assert_eq!(fixture.policy.prompts.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn native_package_grammar_is_checked_before_authorization() {
        let fixture = Fixture::new();
        for spec in ["foo::amd64=1", "foo:amd64:all=1", "foo=~1", "foo:=1"] {
            assert!(
                fixture
                    .owner
                    .owner
                    .begin(":original", "selected", &[spec.into()], ID)
                    .is_err()
            );
        }
        assert!(
            fixture
                .owner
                .owner
                .begin(
                    ":original",
                    "selected",
                    &["foo=1".into(), "foo=2".into()],
                    ID
                )
                .is_err()
        );
        assert_eq!(fixture.policy.prompts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn denied_begin_never_launches_backend() {
        let fixture = Fixture::new();
        fixture.policy.deny.store(true, Ordering::SeqCst);
        assert!(
            fixture
                .owner
                .owner
                .begin(":original", "full", &[], ID)
                .is_err()
        );
        assert!(!fixture.root.join(ID).exists());
        assert!(fixture.owner.owner.state().unwrap().running.is_empty());
    }

    #[test]
    fn approved_plan_and_request_retry_do_not_replay_or_reauthorize() {
        let fixture = Fixture::new();
        let owner = &fixture.owner.owner;
        assert_eq!(owner.begin(":original", "full", &[], ID).unwrap(), ID);
        fixture.wait_status("review");
        assert_eq!(owner.begin(":original", "full", &[], ID).unwrap(), ID);
        assert!(
            owner
                .begin(":original", "selected", &["curl=1".into()], ID)
                .is_err()
        );
        assert!(owner.control(":original", ID, Some(HASH)).unwrap());
        fixture.wait_status("succeeded");
        assert_eq!(owner.begin(":original", "full", &[], ID).unwrap(), ID);
        assert_eq!(fixture.policy.prompts.load(Ordering::SeqCst), 1);
        assert!(
            owner
                .begin(":original", "selected", &["curl=1".into()], ID)
                .is_err()
        );
    }

    #[test]
    fn reconnect_approval_requires_authorization_and_uid_isolated() {
        let fixture = Fixture::new();
        let owner = &fixture.owner.owner;
        owner.begin(":original", "full", &[], ID).unwrap();
        fixture.wait_status("review");
        assert!(owner.status(":other-user", ID).is_err());
        assert!(owner.control(":other-user", ID, None).is_err());
        assert!(owner.recent(":other-user").unwrap().is_empty());
        fixture.policy.deny.store(true, Ordering::SeqCst);
        assert!(owner.control(":reconnected", ID, Some(HASH)).is_err());
        fixture.policy.deny.store(false, Ordering::SeqCst);
        assert!(owner.control(":reconnected", ID, Some(HASH)).unwrap());
        fixture.wait_status("succeeded");
        assert_eq!(fixture.policy.prompts.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn kernel_begin_and_reconnect_use_the_distinct_removal_policy() {
        let fixture = Fixture::new();
        let owner = &fixture.owner.owner;
        let package = ["linux-image-5.4.0-26-generic".to_string()];
        fixture.policy.deny_kernel.store(true, Ordering::SeqCst);
        assert!(owner.begin(":original", "kernel", &package, ID).is_err());
        assert!(!fixture.root.join(ID).exists());
        fixture.policy.deny_kernel.store(false, Ordering::SeqCst);
        owner.begin(":original", "kernel", &package, ID).unwrap();
        fixture.wait_status("review");
        fixture.policy.deny_kernel.store(true, Ordering::SeqCst);
        assert!(owner.control(":reconnected", ID, Some(HASH)).is_err());
        assert!(fixture.wait_status("review").1);
        fixture.policy.deny_kernel.store(false, Ordering::SeqCst);
        assert!(owner.control(":reconnected", ID, Some(HASH)).unwrap());
        fixture.wait_status("succeeded");
    }

    #[test]
    fn cancellation_is_delivery_until_backend_acknowledges() {
        let fixture = Fixture::new();
        let owner = &fixture.owner.owner;
        owner.begin(":original", "full", &[], ID).unwrap();
        assert!(fixture.wait_status("review").1);
        assert!(owner.control(":original", ID, None).unwrap());
        assert!(!fixture.wait_status("cancelled").1);
        assert!(!owner.control(":original", ID, None).unwrap());
    }

    #[test]
    fn another_request_cannot_spawn_second_backend() {
        let fixture = Fixture::new();
        let owner = &fixture.owner.owner;
        owner.begin(":original", "full", &[], ID).unwrap();
        fixture.wait_status("review");
        assert!(
            owner
                .begin(":original", "full", &[], "00000000000000000000000000000000")
                .is_err()
        );
        assert_eq!(fixture.policy.prompts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dead_backend_recovery_is_read_only_and_never_replays() {
        let fixture = Fixture::new();
        let text = format!(
            "VERSION\t1\nID\t{ID}\nUID\t1000\nPID\t2147483647\nSTART\t1\nSTATE\tmutating\nINTENT\tfull\nPLAN\t{HASH}\t0\nEND\n"
        );
        std::fs::write(fixture.root.join(ID), &text).unwrap();
        let (status, control) = fixture.owner.owner.status(":original", ID).unwrap();
        assert!(status.contains("STATE\trecovery_required\n"));
        assert!(!control);
        assert_eq!(
            std::fs::read_to_string(fixture.root.join(ID)).unwrap(),
            text
        );
        assert_eq!(
            fixture
                .owner
                .owner
                .begin(":original", "full", &[], ID)
                .unwrap(),
            ID
        );
        assert_eq!(fixture.policy.prompts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn recovery_audit_is_authorized_read_only_and_preserves_old_receipt_on_startup_failure() {
        let fixture = Fixture::new();
        let original = format!(
            "VERSION\t1\nID\t{ID}\nUID\t1000\nPID\t2147483647\nSTART\t1\nSTATE\tmutating\nINTENT\tfull\nPLAN\t{HASH}\t0\nEND\n"
        );
        std::fs::write(fixture.root.join(ID), &original).unwrap();
        assert!(fixture.owner.owner.reconcile(":other-user", ID).is_err());
        fixture.policy.deny.store(true, Ordering::SeqCst);
        assert!(fixture.owner.owner.reconcile(":original", ID).is_err());
        fixture.policy.deny.store(false, Ordering::SeqCst);
        std::fs::remove_file(fixture.root.join("backend.py")).unwrap();
        fixture.owner.owner.reconcile(":original", ID).unwrap();
        for _ in 0..200 {
            if fixture.owner.owner.state().unwrap().running.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(fixture.owner.owner.state().unwrap().running.is_empty());
        assert_eq!(
            std::fs::read_to_string(fixture.root.join(ID)).unwrap(),
            original
        );
        let script = format!(
            "import pathlib,sys\nassert sys.stdin.readline() == 'RECONCILE\\n'\np=pathlib.Path({:?}) / sys.argv[2]\ns=p.read_text().replace('STATE\\tmutating', 'STATE\\tfailed')\np.write_text(s)\n",
            fixture.root.to_string_lossy()
        );
        std::fs::write(fixture.root.join("backend.py"), script).unwrap();
        fixture.owner.owner.reconcile(":original", ID).unwrap();
        fixture.wait_status("failed");
        assert!(fixture.owner.owner.reconcile(":original", ID).is_err());
    }

    #[test]
    fn receipt_bounds_and_symlinks_fail_closed() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let receipt = fixture.root.join(ID);
        std::fs::write(&receipt, vec![b'x'; MAX_RECEIPT_BYTES as usize + 1]).unwrap();
        assert!(fixture.owner.owner.status(":original", ID).is_err());
        std::fs::remove_file(&receipt).unwrap();
        symlink(fixture.root.join("backend.py"), &receipt).unwrap();
        assert!(fixture.owner.owner.status(":original", ID).is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actual_private_bus_routes_fixed_methods() {
        use std::io::BufRead;
        struct Bus(Child);
        impl Drop for Bus {
            fn drop(&mut self) {
                let _result = self.0.kill();
                let _result = self.0.wait();
            }
        }
        let mut bus = Bus(Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap());
        let mut address = String::new();
        std::io::BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let fixture = Fixture::new();
        let _server = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .name(BUS_NAME)
            .unwrap()
            .serve_at(OBJECT_PATH, fixture.owner.clone())
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        let proxy = zbus::Proxy::new(&client, BUS_NAME, OBJECT_PATH, BUS_NAME)
            .await
            .unwrap();
        let id: String = proxy
            .call("Begin", &("full", Vec::<String>::new(), ID))
            .await
            .unwrap();
        assert_eq!(id, ID);
        fixture.wait_status("review");
        let denied: std::result::Result<(), _> = proxy.call("Reconcile", &(ID,)).await;
        assert!(denied.is_err());
        let (receipt, control): (String, bool) = proxy.call("Status", &(ID,)).await.unwrap();
        assert!(receipt.contains("STATE\treview\n") && control);
        let recent: Vec<String> = proxy.call("Recent", &()).await.unwrap();
        assert_eq!(recent, vec![ID]);
        let accepted: bool = proxy.call("Cancel", &(ID,)).await.unwrap();
        assert!(accepted);
        fixture.wait_status("cancelled");
    }
}

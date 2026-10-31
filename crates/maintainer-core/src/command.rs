use std::collections::HashSet;
use std::io::Read;
use std::process::Command;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use crate::error::{CommandSpecError, MaintainerError};
use crate::jobs::Cancellation;

/// Shell programs that must never be invoked.
static FORBIDDEN_SHELLS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "sh", "bash", "zsh", "dash", "csh", "tcsh", "fish", "ksh", "ash",
    ]
    .into_iter()
    .collect()
});

/// Programs allowed without an absolute path (common Linux utilities).
static ALLOWED_RELATIVE: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "apt",
        "apt-get",
        "apt-cache",
        "dpkg",
        "dpkg-query",
        "systemctl",
        "journalctl",
        "smartctl",
        "lsblk",
        "findmnt",
        "df",
        "du",
        "ps",
        "kill",
        "uname",
        "cat",
        "head",
        "tail",
        "wc",
        "sort",
        "uniq",
        "grep",
        "id",
        "stat",
        "readlink",
        "realpath",
        "flatpak",
        "fwupdmgr",
        "sensors",
        "free",
        "uptime",
        "hostname",
        "ip",
        "ss",
        "mount",
        "umount",
        "blkid",
        "lscpu",
        "lspci",
        "lsusb",
        "nproc",
        "xdg-open",
        "gio",
    ]
    .into_iter()
    .collect()
});

/// Maximum bytes captured from stdout or stderr.
const DEFAULT_OUTPUT_CAP_BYTES: usize = 10 * 1024 * 1024; // 10 MiB
const MAX_STREAM_LINE_BYTES: usize = 64 * 1024;

/// Default command timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// A validated, shell-free command specification.
///
/// Constructed only through `CommandSpec::new`, which rejects shell programs
/// and unrecognized relative paths.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    program: String,
    args: Vec<String>,
    timeout: Duration,
    output_cap_bytes: usize,
}

impl CommandSpec {
    /// Create a new command spec after validation.
    ///
    /// # Errors
    ///
    /// - `CommandSpecError::EmptyProgram` if `program` is empty.
    /// - `CommandSpecError::ForbiddenProgram` if the program is a known shell.
    /// - `CommandSpecError::RelativePath` if the program is not an absolute path
    ///   and is not in the allowlist.
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> std::result::Result<Self, CommandSpecError> {
        let program = program.into();
        if program.is_empty() {
            return Err(CommandSpecError::EmptyProgram);
        }

        let basename = program.rsplit('/').next().unwrap_or(&program);

        if FORBIDDEN_SHELLS.contains(basename) {
            return Err(CommandSpecError::ForbiddenProgram(program));
        }

        // If it's not an absolute path, it must be in the allowlist.
        if !program.starts_with('/') && !ALLOWED_RELATIVE.contains(program.as_str()) {
            return Err(CommandSpecError::RelativePath(program));
        }

        Ok(Self {
            program,
            args: args.into_iter().map(Into::into).collect(),
            timeout: DEFAULT_TIMEOUT,
            output_cap_bytes: DEFAULT_OUTPUT_CAP_BYTES,
        })
    }

    /// Override the default timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Override the default output capture cap.
    pub fn with_output_cap(mut self, bytes: usize) -> Self {
        self.output_cap_bytes = bytes;
        self
    }

    /// The program path or name.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The argument list.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// The configured timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The output capture cap in bytes.
    pub fn output_cap_bytes(&self) -> usize {
        self.output_cap_bytes
    }
}

/// Result of running a command.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// Process exit code, if the process exited normally.
    pub exit_code: Option<i32>,
    /// Captured stdout (truncated to output cap).
    pub stdout: String,
    /// Captured stderr (truncated to output cap).
    pub stderr: String,
    /// Whether stdout was truncated due to the output cap.
    pub stdout_truncated: bool,
    /// Whether stderr was truncated due to the output cap.
    pub stderr_truncated: bool,
}

impl CommandOutput {
    /// Returns `true` if the command exited with code 0.
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// Trait for executing validated commands without a shell.
pub trait CommandRunner: Send + Sync {
    /// Run the command, checking for cancellation. Returns structured output.
    fn run(
        &self,
        spec: &CommandSpec,
        cancel: &dyn Cancellation,
    ) -> crate::error::Result<CommandOutput>;
}

/// Standard implementation that uses `std::process::Command`.
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(
        &self,
        spec: &CommandSpec,
        cancel: &dyn Cancellation,
    ) -> crate::error::Result<CommandOutput> {
        run_lifecycle(spec, cancel, None)
    }
}

struct ChildLifecycle {
    child: std::process::Child,
    grouped: bool,
    armed: bool,
}

impl ChildLifecycle {
    fn terminate(&mut self) {
        if self.grouped {
            let _ = terminate_process_group(self.child.id());
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ChildLifecycle {
    fn drop(&mut self) {
        if self.armed {
            self.terminate();
        }
    }
}

fn command_failure(message: &str) -> MaintainerError {
    MaintainerError::CommandFailed {
        message: message.to_owned(),
    }
}

fn run_lifecycle(
    spec: &CommandSpec,
    cancel: &dyn Cancellation,
    mut on_line: Option<&mut dyn FnMut(&str)>,
) -> crate::error::Result<CommandOutput> {
    use std::os::unix::process::CommandExt;
    use std::sync::mpsc;
    if cancel.is_cancelled() {
        return Err(MaintainerError::Cancelled);
    }
    let streaming = on_line.is_some();
    // Elevated transaction cancellation retains its existing direct-child behavior.
    let grouped = spec.program().rsplit('/').next() != Some("pkexec");
    let mut command = Command::new(spec.program());
    command
        .args(spec.args())
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if grouped {
        command.process_group(0);
    }
    let child = command
        .spawn()
        .map_err(|error| MaintainerError::io("spawning command", error))?;
    let mut child = ChildLifecycle {
        child,
        grouped,
        armed: true,
    };
    let start = Instant::now();
    let readers = ReaderControl::new();
    let stdout = child
        .child
        .stdout
        .take()
        .ok_or_else(|| command_failure("command stdout pipe unavailable"))?;
    let stderr = child
        .child
        .stderr
        .take()
        .ok_or_else(|| command_failure("command stderr pipe unavailable"))?;
    let stdout = readers
        .wrap(stdout)
        .map_err(|error| MaintainerError::io("configuring stdout capture", error))?;
    let stderr = readers
        .wrap(stderr)
        .map_err(|error| MaintainerError::io("configuring stderr capture", error))?;
    let cap = spec.output_cap_bytes().min(DEFAULT_OUTPUT_CAP_BYTES);
    let (line_tx, line_rx) = mpsc::sync_channel(64);
    let stdout_reader = std::thread::Builder::new()
        .name("command-stdout".into())
        .spawn(move || {
            if streaming {
                read_stream_lines(stdout, cap, MAX_STREAM_LINE_BYTES, &line_tx)
            } else {
                read_capped(stdout, cap)
            }
        })
        .map_err(|error| MaintainerError::io("starting stdout capture", error))?;
    let stderr_reader = match std::thread::Builder::new()
        .name("command-stderr".into())
        .spawn(move || read_capped(stderr, cap))
    {
        Ok(reader) => reader,
        Err(error) => {
            drop(line_rx);
            readers.stop();
            child.terminate();
            let _ = stdout_reader.join();
            return Err(MaintainerError::io("starting stderr capture", error));
        }
    };
    let outcome = (|| {
        let mut status = None;
        let mut channel_closed = !streaming;
        loop {
            if cancel.is_cancelled() {
                return Err(MaintainerError::Cancelled);
            }
            if start.elapsed() >= spec.timeout() {
                return Err(command_failure(
                    "command timed out before process and capture completed",
                ));
            }
            if status.is_none() {
                status = child
                    .child
                    .try_wait()
                    .map_err(|error| MaintainerError::io("waiting for command", error))?;
            }
            if streaming && !channel_closed {
                match line_rx.recv_timeout(Duration::from_millis(5)) {
                    Ok(Ok(line)) => {
                        if let Some(callback) = on_line.as_mut() {
                            callback(&line);
                        }
                    }
                    Ok(Err(message)) => {
                        return Err(MaintainerError::invalid_data("command output", message));
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => channel_closed = true,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
            if let Some(status) = status {
                if channel_closed && stdout_reader.is_finished() && stderr_reader.is_finished() {
                    return Ok(status);
                }
            }
        }
    })();
    // Release channel backpressure before stopping/joining a streaming reader.
    drop(line_rx);
    if outcome.is_err() {
        child.terminate();
        readers.stop();
    }
    let stdout_result = stdout_reader.join();
    let stderr_result = stderr_reader.join();
    child.armed = false;
    let status = outcome?;
    let (stdout_bytes, stdout_truncated) = stdout_result
        .map_err(|_| command_failure("stdout capture stopped unexpectedly"))?
        .map_err(|error| MaintainerError::io("reading stdout", error))?;
    let (stderr_bytes, stderr_truncated) = stderr_result
        .map_err(|_| command_failure("stderr capture stopped unexpectedly"))?
        .map_err(|error| MaintainerError::io("reading stderr", error))?;
    if streaming && stdout_truncated {
        return Err(MaintainerError::invalid_data(
            "command output",
            "stdout exceeded capture limit",
        ));
    }
    // Raw diagnostics are lossy text; streaming protocol lines are validated UTF-8.
    Ok(CommandOutput {
        exit_code: status.code(),
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        stdout_truncated,
        stderr_truncated,
    })
}

fn terminate_process_group(child_pid: u32) -> std::io::Result<()> {
    // The child is started as its own process-group leader, so its negative PID
    // cannot target the GUI or unrelated processes.
    let result = unsafe { libc::kill(-(child_pid as libc::pid_t), libc::SIGKILL) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

struct ReaderControl(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl ReaderControl {
    fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )))
    }

    fn stop(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    fn wrap<R: Read + std::os::fd::AsRawFd>(
        &self,
        reader: R,
    ) -> std::io::Result<InterruptibleReader<R>> {
        // SAFETY: the owned pipe descriptor is valid; only its nonblocking flag changes.
        let flags = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };
        if flags == -1
            || unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                == -1
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(InterruptibleReader {
            reader,
            stop: self.0.clone(),
        })
    }
}

impl Drop for ReaderControl {
    fn drop(&mut self) {
        self.stop();
    }
}

struct InterruptibleReader<R> {
    reader: R,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl<R: Read> Read for InterruptibleReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.stop.load(std::sync::atomic::Ordering::Acquire) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "command capture stopped",
                ));
            }
            match self.reader.read(buffer) {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
}

fn read_capped(mut reader: impl Read, cap: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut captured = Vec::with_capacity(cap.min(64 * 1024));
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            return Ok((captured, truncated));
        }
        let remaining = cap.saturating_sub(captured.len());
        let captured_bytes = bytes_read.min(remaining);
        captured.extend_from_slice(&buffer[..captured_bytes]);
        truncated |= captured_bytes < bytes_read;
    }
}

fn read_stream_lines(
    mut reader: impl Read,
    output_cap: usize,
    line_cap: usize,
    sender: &std::sync::mpsc::SyncSender<std::result::Result<String, String>>,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::with_capacity(output_cap.min(64 * 1024));
    let mut line = Vec::with_capacity(line_cap.min(4096));
    let mut buffer = [0_u8; 8192];
    let mut output_truncated = false;
    let mut line_overflow = false;

    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            if !line.is_empty() && !line_overflow {
                match String::from_utf8(line.clone()) {
                    Ok(text) => {
                        let _ = sender.send(Ok(text));
                    }
                    Err(_) => {
                        let _ = sender.send(Err("stdout protocol contains invalid UTF-8".into()));
                        return Ok((output, output_truncated));
                    }
                }
            }
            return Ok((output, output_truncated));
        }

        let remaining = output_cap.saturating_sub(output.len());
        let captured = count.min(remaining);
        output.extend_from_slice(&buffer[..captured]);
        output_truncated |= captured < count;

        for byte in &buffer[..count] {
            if *byte == b'\n' {
                if !line_overflow {
                    match String::from_utf8(line.clone()) {
                        Ok(text) => {
                            let _ = sender.send(Ok(text));
                        }
                        Err(_) => {
                            let _ =
                                sender.send(Err("stdout protocol contains invalid UTF-8".into()));
                            return Ok((output, output_truncated));
                        }
                    }
                }
                line.clear();
                line_overflow = false;
            } else if line.len() < line_cap {
                line.push(*byte);
            } else if !line_overflow {
                let _ = sender.send(Err(format!(
                    "stdout line exceeded the {line_cap}-byte limit"
                )));
                line.clear();
                line_overflow = true;
            }
        }
    }
}

/// Run a validated `CommandSpec` while streaming stdout line-by-line to a callback.
///
/// This is a free function (not a trait method) to avoid changing the
/// `CommandRunner` trait signature that many callers and mock implementations
/// depend on. It reuses the already-validated `CommandSpec` — the same
/// allowlist, absolute-path, and no-shell-strings guarantees apply.
///
/// # Behavior
///
/// - Spawns the process with piped stdout and stderr.
/// - Reads stdout line-by-line in a background thread, forwarding lines
///   through a channel to the calling thread.
/// - The calling thread polls the channel, checking cancellation and timeout
///   between polls — so even a process that blocks on stdout for a long time
///   will be killed promptly on cancel or timeout.
/// - Stderr is collected in a second background thread to avoid deadlock.
/// - Returns a `CommandOutput` with the full collected stdout and stderr,
///   identical to what `SystemCommandRunner::run()` would produce, so
///   callers can still inspect the final output for exit-code classification,
///   restart detection, etc.
///
/// # Safety invariants preserved
///
/// - `CommandSpec` validation is not bypassed (caller must pass an already-
///   validated spec).
/// - No shell interpretation: `Command::new` with explicit args.
/// - `LC_ALL=C` environment, null stdin.
pub fn run_streaming(
    spec: &CommandSpec,
    cancel: &dyn Cancellation,
    mut on_line: impl FnMut(&str),
) -> crate::error::Result<CommandOutput> {
    run_lifecycle(spec, cancel, Some(&mut on_line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::CancellationToken;

    #[test]
    fn command_spec_rejects_shell_execution() {
        let result = CommandSpec::new("sh", ["-c", "rm -rf /tmp/example"]);
        assert!(matches!(result, Err(CommandSpecError::ForbiddenProgram(_))));
    }

    #[test]
    fn command_spec_rejects_bash() {
        let result = CommandSpec::new("bash", ["-c", "echo hello"]);
        assert!(matches!(result, Err(CommandSpecError::ForbiddenProgram(_))));
    }

    #[test]
    fn command_spec_rejects_zsh() {
        let result = CommandSpec::new("zsh", ["-c", "echo hello"]);
        assert!(matches!(result, Err(CommandSpecError::ForbiddenProgram(_))));
    }

    #[test]
    fn command_spec_rejects_dash() {
        let result = CommandSpec::new("dash", ["-c", "echo hello"]);
        assert!(matches!(result, Err(CommandSpecError::ForbiddenProgram(_))));
    }

    #[test]
    fn command_spec_rejects_fish() {
        let result = CommandSpec::new("fish", ["-c", "echo hello"]);
        assert!(matches!(result, Err(CommandSpecError::ForbiddenProgram(_))));
    }

    #[test]
    fn command_spec_rejects_csh() {
        let result = CommandSpec::new("csh", ["-c", "echo hello"]);
        assert!(matches!(result, Err(CommandSpecError::ForbiddenProgram(_))));
    }

    #[test]
    fn command_spec_rejects_absolute_path_to_shell() {
        let result = CommandSpec::new("/bin/bash", ["-c", "echo hello"]);
        assert!(matches!(result, Err(CommandSpecError::ForbiddenProgram(_))));
    }

    #[test]
    fn command_spec_rejects_usr_bin_shell() {
        let result = CommandSpec::new("/usr/bin/zsh", ["-c", "echo hello"]);
        assert!(matches!(result, Err(CommandSpecError::ForbiddenProgram(_))));
    }

    #[test]
    fn command_spec_rejects_empty_program() {
        let result = CommandSpec::new("", Vec::<String>::new());
        assert!(matches!(result, Err(CommandSpecError::EmptyProgram)));
    }

    #[test]
    fn command_spec_rejects_unknown_relative_program() {
        let result = CommandSpec::new("my-custom-tool", ["--flag"]);
        assert!(matches!(result, Err(CommandSpecError::RelativePath(_))));
    }

    #[test]
    fn command_spec_allows_absolute_path() {
        let result = CommandSpec::new("/usr/bin/lsblk", ["--json"]);
        assert!(result.is_ok());
        let spec = result.unwrap();
        assert_eq!(spec.program(), "/usr/bin/lsblk");
        assert_eq!(spec.args(), &["--json"]);
    }

    #[test]
    fn command_spec_allows_allowlisted_relative() {
        let result = CommandSpec::new("systemctl", ["status", "NetworkManager"]);
        assert!(result.is_ok());
        let spec = result.unwrap();
        assert_eq!(spec.program(), "systemctl");
    }

    #[test]
    fn command_spec_preserves_arguments_exactly() {
        let result = CommandSpec::new("df", ["-h", "--output=source,size,used,avail"]);
        assert!(result.is_ok());
        let spec = result.unwrap();
        assert_eq!(spec.args(), &["-h", "--output=source,size,used,avail"]);
    }

    #[test]
    fn command_spec_custom_timeout() {
        let spec = CommandSpec::new("df", ["-h"])
            .unwrap()
            .with_timeout(Duration::from_secs(5));
        assert_eq!(spec.timeout(), Duration::from_secs(5));
    }

    #[test]
    fn command_spec_custom_output_cap() {
        let spec = CommandSpec::new("df", ["-h"])
            .unwrap()
            .with_output_cap(1024);
        assert_eq!(spec.output_cap_bytes(), 1024);
    }

    #[test]
    fn command_runner_executes_true() {
        let runner = SystemCommandRunner;
        let spec = CommandSpec::new("/usr/bin/true", Vec::<String>::new()).unwrap();
        let token = CancellationToken::new();
        let result = runner.run(&spec, &token);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.success());
        assert_eq!(output.exit_code, Some(0));
    }

    #[test]
    fn command_runner_captures_stdout() {
        let runner = SystemCommandRunner;
        let spec = CommandSpec::new("/bin/echo", ["hello world"]).unwrap();
        let token = CancellationToken::new();
        let result = runner.run(&spec, &token).unwrap();
        assert!(result.success());
        assert_eq!(result.stdout.trim(), "hello world");
    }

    #[test]
    fn command_runner_reports_nonzero_exit() {
        let runner = SystemCommandRunner;
        let spec = CommandSpec::new("/usr/bin/false", Vec::<String>::new()).unwrap();
        let token = CancellationToken::new();
        let result = runner.run(&spec, &token).unwrap();
        assert!(!result.success());
        assert_ne!(result.exit_code, Some(0));
    }

    #[test]
    fn command_runner_sets_lc_all_c() {
        let runner = SystemCommandRunner;
        let spec = CommandSpec::new("/usr/bin/env", Vec::<String>::new()).unwrap();
        let token = CancellationToken::new();
        let result = runner.run(&spec, &token).unwrap();
        assert!(result.stdout.contains("LC_ALL=C"));
    }

    #[test]
    fn command_runner_cancellation_before_start() {
        let runner = SystemCommandRunner;
        let spec = CommandSpec::new("/bin/sleep", ["10"]).unwrap();
        let token = CancellationToken::new();
        token.cancel();
        let result = runner.run(&spec, &token);
        assert!(matches!(result, Err(MaintainerError::Cancelled)));
    }

    #[test]
    fn command_runner_timeout() {
        let runner = SystemCommandRunner;
        let spec = CommandSpec::new("/bin/sleep", ["10"])
            .unwrap()
            .with_timeout(Duration::from_millis(100));
        let token = CancellationToken::new();
        let result = runner.run(&spec, &token);
        assert!(matches!(result, Err(MaintainerError::CommandFailed { .. })));
    }

    #[test]
    fn command_runner_timeout_kills_descendant_holding_pipe() {
        let spec = CommandSpec::new(
            "/usr/bin/python3",
            [
                "-c",
                "import subprocess,time; subprocess.Popen(['/bin/sleep','10']); print('ready', flush=True); time.sleep(10)",
            ],
        )
        .unwrap()
        .with_timeout(Duration::from_millis(200));
        let token = CancellationToken::new();
        let started = Instant::now();

        let result = SystemCommandRunner.run(&spec, &token);

        assert!(matches!(result, Err(MaintainerError::CommandFailed { .. })));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn command_runner_output_cap_truncates() {
        let runner = SystemCommandRunner;
        let spec = CommandSpec::new("/usr/bin/yes", Vec::<String>::new())
            .unwrap()
            .with_output_cap(100)
            .with_timeout(Duration::from_millis(200));
        let token = CancellationToken::new();
        let result = runner.run(&spec, &token);
        match result {
            Ok(output) => {
                assert!(output.stdout.len() <= 100 || output.stdout_truncated);
            }
            Err(MaintainerError::CommandFailed { .. }) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn capped_reader_drains_past_capture_limit() {
        let input = vec![b'x'; 1024];
        let (captured, truncated) = read_capped(input.as_slice(), 16).unwrap();
        assert_eq!(captured, vec![b'x'; 16]);
        assert!(truncated);
    }

    #[test]
    fn relative_directory_cannot_bypass_program_allowlist() {
        assert!(CommandSpec::new("untrusted/apt", ["--version"]).is_err());
        assert!(CommandSpec::new("./apt", ["--version"]).is_err());
    }

    #[test]
    fn descendant_retained_pipes_obey_capture_deadline() {
        for streaming in [false, true] {
            let spec = CommandSpec::new("/usr/bin/python3", ["-c", "import os,time; child=os.fork(); os.close(1) if child == 0 else None; time.sleep(1) if child == 0 else None; os._exit(0)"])
                .unwrap().with_timeout(Duration::from_millis(100));
            let started = Instant::now();
            let result = if streaming {
                run_streaming(&spec, &CancellationToken::new(), |_| {})
            } else {
                SystemCommandRunner.run(&spec, &CancellationToken::new())
            };
            assert!(result.is_err());
            assert!(started.elapsed() < Duration::from_millis(700));
        }
    }

    #[test]
    fn both_capture_modes_drain_both_pipes_past_capacity() {
        for streaming in [false, true] {
            let spec = CommandSpec::new("/usr/bin/python3", ["-c", "import os,threading; t=threading.Thread(target=lambda: [os.write(2,b'e'*4096) for _ in range(512)]); t.start(); [os.write(1,b'x'*1023+b'\\n') for _ in range(2048)]; t.join()"])
                .unwrap().with_timeout(Duration::from_secs(5)).with_output_cap(8192);
            let started = Instant::now();
            if streaming {
                assert!(run_streaming(&spec, &CancellationToken::new(), |_| {}).is_err());
            } else {
                let output = SystemCommandRunner
                    .run(&spec, &CancellationToken::new())
                    .unwrap();
                assert!(output.success());
                assert!(output.stdout_truncated && output.stderr_truncated);
                assert_eq!(output.stdout.len(), 8192);
                assert_eq!(output.stderr.len(), 8192);
            }
            assert!(started.elapsed() < Duration::from_secs(5));
        }
    }

    #[test]
    fn streaming_rejects_invalid_utf8_while_raw_diagnostics_are_lossy() {
        let spec = CommandSpec::new(
            "/usr/bin/python3",
            ["-c", "import os; os.write(1,bytes([255,10]))"],
        )
        .unwrap();
        assert!(matches!(
            run_streaming(&spec, &CancellationToken::new(), |_| {}),
            Err(MaintainerError::InvalidData { .. })
        ));
        let output = SystemCommandRunner
            .run(&spec, &CancellationToken::new())
            .unwrap();
        assert!(output.stdout.contains('\u{fffd}'));
    }

    #[test]
    fn run_streaming_delivers_lines_incrementally() {
        let spec = CommandSpec::new("/usr/bin/printf", ["line1\\nline2\\nline3\\n"]).unwrap();
        let token = CancellationToken::new();
        let mut lines_received = Vec::new();
        let result = run_streaming(&spec, &token, |line| {
            lines_received.push(line.to_string());
        });
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.success());
        assert_eq!(lines_received, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn run_streaming_cancellation_before_start() {
        let spec = CommandSpec::new("/bin/sleep", ["10"]).unwrap();
        let token = CancellationToken::new();
        token.cancel();
        let result = run_streaming(&spec, &token, |_| {});
        assert!(matches!(result, Err(MaintainerError::Cancelled)));
    }

    #[test]
    fn run_streaming_timeout() {
        let spec = CommandSpec::new("/bin/sleep", ["10"])
            .unwrap()
            .with_timeout(Duration::from_millis(100));
        let token = CancellationToken::new();
        let result = run_streaming(&spec, &token, |_| {});
        assert!(matches!(result, Err(MaintainerError::CommandFailed { .. })));
    }

    #[test]
    fn run_streaming_timeout_still_applies_after_stdout_eof() {
        let spec = CommandSpec::new(
            "/usr/bin/python3",
            ["-c", "import os,time; os.close(1); time.sleep(10)"],
        )
        .unwrap()
        .with_timeout(Duration::from_millis(200));
        let token = CancellationToken::new();
        let started = Instant::now();

        let result = run_streaming(&spec, &token, |_| {});

        assert!(matches!(result, Err(MaintainerError::CommandFailed { .. })));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn run_streaming_rejects_newline_free_output_over_line_limit() {
        let spec = CommandSpec::new(
            "/usr/bin/python3",
            [
                "-c",
                "import sys; [sys.stdout.write('x' * 65536) for _ in range(512)]",
            ],
        )
        .unwrap()
        .with_timeout(Duration::from_secs(3));
        let token = CancellationToken::new();

        let result = run_streaming(&spec, &token, |_| {});

        assert!(matches!(result, Err(MaintainerError::InvalidData { .. })));
    }

    #[test]
    fn run_streaming_captures_full_output() {
        let spec = CommandSpec::new("/bin/echo", ["hello streaming"]).unwrap();
        let token = CancellationToken::new();
        let mut lines = Vec::new();
        let result = run_streaming(&spec, &token, |line| {
            lines.push(line.to_string());
        })
        .unwrap();
        assert!(result.success());
        // Full stdout is also available in the returned CommandOutput
        assert_eq!(result.stdout.trim(), "hello streaming");
        assert_eq!(lines, vec!["hello streaming"]);
    }

    #[test]
    fn run_streaming_reports_nonzero_exit() {
        let spec = CommandSpec::new("/usr/bin/false", Vec::<String>::new()).unwrap();
        let token = CancellationToken::new();
        let result = run_streaming(&spec, &token, |_| {}).unwrap();
        assert!(!result.success());
    }

    /// Proves that `run_streaming` delivers multiple progress callbacks during
    /// a single command invocation — the core property needed for incremental
    /// update progress reporting.
    #[test]
    fn run_streaming_multiple_progress_reports_during_execution() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        // /usr/bin/printf emits multiple lines — each triggers a callback.
        // Simulate pkcon-like output with multiple Status/Percentage lines.
        let pkcon_like_output = concat!(
            "Status: \\tWaiting in queue\\n",
            "Status: \\tStarting\\n",
            "Percentage:\\t0\\n",
            "Status: \\tLoading cache\\n",
            "Percentage:\\t50\\n",
            "Status: \\tInstalling packages\\n",
            "Package:\\tcurl-7.81.0-1ubuntu1.16.amd64\\n",
            "Percentage:\\t100\\n",
            "Status: \\tFinished\\n",
        );

        let spec = CommandSpec::new("/usr/bin/printf", [pkcon_like_output]).unwrap();
        let token = CancellationToken::new();
        let report_count = Arc::new(AtomicU32::new(0));
        let report_count_clone = report_count.clone();

        let result = run_streaming(&spec, &token, |line| {
            // Parse each line the same way dispatch_update_execution does.
            use crate::updates::pkcon_parser::parse_pkcon_plain_line;
            if parse_pkcon_plain_line(line).is_some() {
                report_count_clone.fetch_add(1, Ordering::Relaxed);
            }
        });

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.success());

        // Must have received multiple progress-worthy lines (not just one).
        let count = report_count.load(Ordering::Relaxed);
        assert!(
            count >= 5,
            "Expected at least 5 progress-worthy lines, got {count}"
        );
    }
}

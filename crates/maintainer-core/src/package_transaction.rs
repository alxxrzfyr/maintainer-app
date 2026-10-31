//! Bounded package-owner receipts shared by the unprivileged client and helper.

use std::collections::HashSet;

pub const MAX_RECEIPT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_OPERATIONS: usize = 4096;
pub const MAX_REQUESTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackagePhase {
    Locking,
    Resolving,
    Review,
    Fetching,
    Mutating,
    Succeeded,
    Cancelled,
    Failed,
    RecoveryRequired,
}

impl PackagePhase {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Cancelled | Self::Failed | Self::RecoveryRequired
        )
    }

    pub fn can_cancel(self) -> bool {
        matches!(
            self,
            Self::Locking | Self::Resolving | Self::Review | Self::Fetching
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageChange {
    pub operation: String,
    pub name: String,
    pub architecture: String,
    pub old_version: Option<String>,
    pub new_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageReceipt {
    pub id: String,
    pub uid: u32,
    pub backend_pid: u32,
    pub backend_start_ticks: u64,
    pub phase: PackagePhase,
    pub intent: String,
    pub requests: Vec<String>,
    pub fingerprint: Option<String>,
    pub download_bytes: u64,
    pub changes: Vec<PackageChange>,
    pub error_code: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid or incomplete package transaction receipt")]
pub struct ReceiptError;

/// Generate a request identity once; retries and reconnection retain this ID.
pub fn new_transaction_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub fn valid_transaction_id(id: &str) -> bool {
    hex(id, 32)
}

fn hex(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn token(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && value.bytes().all(|b| b.is_ascii_graphic())
}

fn phase(value: &str) -> Result<PackagePhase, ReceiptError> {
    Ok(match value {
        "locking" => PackagePhase::Locking,
        "resolving" => PackagePhase::Resolving,
        "review" => PackagePhase::Review,
        "fetching" => PackagePhase::Fetching,
        "mutating" => PackagePhase::Mutating,
        "succeeded" => PackagePhase::Succeeded,
        "cancelled" => PackagePhase::Cancelled,
        "failed" => PackagePhase::Failed,
        "recovery_required" => PackagePhase::RecoveryRequired,
        _ => return Err(ReceiptError),
    })
}

impl PackageReceipt {
    /// Unknown fields, duplicate headers and truncated records fail closed.
    pub fn parse(text: &str) -> Result<Self, ReceiptError> {
        if text.len() > MAX_RECEIPT_BYTES || !text.is_ascii() || !text.ends_with("END\n") {
            return Err(ReceiptError);
        }
        let mut headers = std::collections::HashMap::new();
        let mut requests = Vec::new();
        let mut changes = Vec::new();
        let mut identities = HashSet::new();
        let mut ended = false;
        for line in text.lines() {
            if ended {
                return Err(ReceiptError);
            }
            let fields: Vec<_> = line.split('\t').collect();
            match fields.as_slice() {
                ["END"] => ended = true,
                ["REQUEST", value] if token(value, 1024) && requests.len() < MAX_REQUESTS => {
                    if requests.iter().any(|request| request == value) {
                        return Err(ReceiptError);
                    }
                    requests.push((*value).to_owned());
                }
                ["ITEM", operation, name, architecture, old, new]
                    if changes.len() < MAX_OPERATIONS
                        && matches!(
                            *operation,
                            "install" | "upgrade" | "downgrade" | "reinstall" | "remove"
                        )
                        && token(name, 256)
                        && token(architecture, 64)
                        && token(old, 512)
                        && token(new, 512) =>
                {
                    if !identities.insert(((*name).to_owned(), (*architecture).to_owned()))
                        || (*operation == "remove" && (*old == "-" || *new != "-"))
                        || (*operation != "remove" && *new == "-")
                    {
                        return Err(ReceiptError);
                    }
                    changes.push(PackageChange {
                        operation: (*operation).to_owned(),
                        name: (*name).to_owned(),
                        architecture: (*architecture).to_owned(),
                        old_version: (*old != "-").then(|| (*old).to_owned()),
                        new_version: (*new != "-").then(|| (*new).to_owned()),
                    });
                }
                ["PLAN", fingerprint, bytes] if (*fingerprint == "-" || hex(fingerprint, 64)) => {
                    if headers.insert("PLAN", *fingerprint).is_some()
                        || headers.insert("BYTES", *bytes).is_some()
                    {
                        return Err(ReceiptError);
                    }
                }
                [key, value]
                    if matches!(
                        *key,
                        "VERSION" | "ID" | "UID" | "PID" | "START" | "STATE" | "INTENT" | "ERROR"
                    ) && token(value, 128) =>
                {
                    if headers.insert(*key, *value).is_some() {
                        return Err(ReceiptError);
                    }
                }
                _ => return Err(ReceiptError),
            }
        }
        let field = |name| headers.get(name).copied().ok_or(ReceiptError);
        if field("VERSION")? != "1" || !valid_transaction_id(field("ID")?) {
            return Err(ReceiptError);
        }
        let intent = field("INTENT")?;
        if !matches!(intent, "full" | "selected" | "kernel")
            || (intent == "full" && !requests.is_empty())
            || (intent != "full" && requests.is_empty())
        {
            return Err(ReceiptError);
        }
        let backend_pid = field("PID")?.parse::<u32>().map_err(|_| ReceiptError)?;
        let backend_start_ticks = field("START")?.parse::<u64>().map_err(|_| ReceiptError)?;
        if backend_pid < 2 || backend_pid > libc::pid_t::MAX as u32 || backend_start_ticks == 0 {
            return Err(ReceiptError);
        }
        let phase = phase(field("STATE")?)?;
        let fingerprint = field("PLAN")?;
        if matches!(
            phase,
            PackagePhase::Review
                | PackagePhase::Fetching
                | PackagePhase::Mutating
                | PackagePhase::Succeeded
        ) && fingerprint == "-"
        {
            return Err(ReceiptError);
        }
        Ok(Self {
            id: field("ID")?.to_owned(),
            uid: field("UID")?.parse().map_err(|_| ReceiptError)?,
            backend_pid,
            backend_start_ticks,
            phase,
            intent: intent.to_owned(),
            requests,
            fingerprint: (fingerprint != "-").then(|| fingerprint.to_owned()),
            download_bytes: field("BYTES")?.parse().map_err(|_| ReceiptError)?,
            changes,
            error_code: headers.get("ERROR").map(|s| (*s).to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt() -> String {
        format!(
            "VERSION\t1\nID\t{}\nUID\t1000\nPID\t42\nSTART\t100\nINTENT\tselected\nREQUEST\tcurl=2\nSTATE\treview\nPLAN\t{}\t1024\nITEM\tupgrade\tcurl\tamd64\t1\t2\nEND\n",
            "a".repeat(32),
            "b".repeat(64)
        )
    }

    #[test]
    fn bounded_typed_review_preserves_complete_operations() {
        let parsed = PackageReceipt::parse(&receipt()).unwrap();
        assert_eq!(parsed.uid, 1000);
        assert_eq!(parsed.phase, PackagePhase::Review);
        assert!(parsed.phase.can_cancel());
        assert_eq!(parsed.download_bytes, 1024);
        assert_eq!(parsed.changes[0].old_version.as_deref(), Some("1"));
        assert_eq!(parsed.changes[0].new_version.as_deref(), Some("2"));
    }

    #[test]
    fn malformed_truncated_duplicate_and_oversized_receipts_fail() {
        let original = receipt();
        for invalid in [
            original.replace("END\n", ""),
            original.replace("UID\t1000", "UID\t1000\nUID\t1"),
            original.replace("STATE\treview", "STATE\tunknown"),
            original.replace("PID\t42", "PID\t4294967295"),
            original.replace("ITEM\tupgrade", "ITEM\tpurge"),
            original.replace("END\n", "END\nSTATE\tsucceeded\n"),
            "x".repeat(MAX_RECEIPT_BYTES + 1),
        ] {
            assert!(PackageReceipt::parse(&invalid).is_err());
        }
    }

    #[test]
    fn mutation_and_terminal_states_do_not_promise_cancellation() {
        for state in [
            "mutating",
            "succeeded",
            "failed",
            "cancelled",
            "recovery_required",
        ] {
            let text = receipt().replace("STATE\treview", &format!("STATE\t{state}"));
            let parsed = PackageReceipt::parse(&text).unwrap();
            assert!(!parsed.phase.can_cancel());
            assert_eq!(parsed.phase.is_terminal(), state != "mutating");
        }
    }
}

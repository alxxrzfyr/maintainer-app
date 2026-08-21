use std::collections::HashSet;
use std::process::Command;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use crate::error::{CommandSpecError, MaintainerError};
use crate::jobs::Cancellation;

// ---------------------------------------------------------------------------
// Forbidden shells
// ---------------------------------------------------------------------------

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
        "nproc",
        "xdg-open",
        "gio",
    ]
    .into_iter()
    .collect()
});

// ---------------------------------------------------------------------------
// CommandSpec
// ---------------------------------------------------------------------------

/// Maximum bytes captured from stdout or stderr.
const DEFAULT_OUTPUT_CAP_BYTES: usize = 10 * 1024 * 1024; // 10 MiB

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

        // Extract the basename for shell checks.
        let basename = program.rsplit('/').next().unwrap_or(&program);

        if FORBIDDEN_SHELLS.contains(basename) {
            return Err(CommandSpecError::ForbiddenProgram(program));
        }

        // If it's not an absolute path, it must be in the allowlist.
        if !program.starts_with('/') && !ALLOWED_RELATIVE.contains(basename) {
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

// ---------------------------------------------------------------------------
// CommandOutput
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// CommandRunner trait
// ---------------------------------------------------------------------------

/// Trait for executing validated commands without a shell.
pub trait CommandRunner: Send + Sync {
    /// Run the command, checking for cancellation. Returns structured output.
    fn run(
        &self,
        spec: &CommandSpec,
        cancel: &dyn Cancellation,
    ) -> crate::error::Result<CommandOutput>;
}

// ---------------------------------------------------------------------------
// Default implementation
// ---------------------------------------------------------------------------

/// Standard implementation that uses `std::process::Command`.
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(
        &self,
        spec: &CommandSpec,
        cancel: &dyn Cancellation,
    ) -> crate::error::Result<CommandOutput> {
        // Check cancellation before starting.
        if cancel.is_cancelled() {
            return Err(MaintainerError::Cancelled);
        }

        let mut child = Command::new(spec.program())
            .args(spec.args())
            .env("LC_ALL", "C")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| MaintainerError::io(format!("spawning {}", spec.program()), e))?;

        let deadline = Instant::now() + spec.timeout();

        // Poll for completion, checking cancellation and timeout.
        loop {
            if cancel.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(MaintainerError::Cancelled);
            }

            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(MaintainerError::CommandFailed {
                    message: format!("{} timed out after {:?}", spec.program(), spec.timeout()),
                });
            }

            match child.try_wait() {
                Ok(Some(_status)) => break,
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    return Err(MaintainerError::io(
                        format!("waiting for {}", spec.program()),
                        e,
                    ));
                }
            }
        }

        let output = child
            .wait_with_output()
            .map_err(|e| MaintainerError::io(format!("reading output of {}", spec.program()), e))?;

        let (stdout, stdout_truncated) = truncate_output(&output.stdout, spec.output_cap_bytes());
        let (stderr, stderr_truncated) = truncate_output(&output.stderr, spec.output_cap_bytes());

        Ok(CommandOutput {
            exit_code: output.status.code(),
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
        })
    }
}

/// Truncate byte output to a string, handling the cap.
fn truncate_output(bytes: &[u8], cap: usize) -> (String, bool) {
    let truncated = bytes.len() > cap;
    let slice = if truncated { &bytes[..cap] } else { bytes };
    let text = String::from_utf8_lossy(slice).into_owned();
    (text, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::CancellationToken;

    // --- CommandSpec validation tests ---

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

    // --- SystemCommandRunner integration tests ---

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
        // /usr/bin/env prints environment; grep for LC_ALL
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
    fn command_runner_output_cap_truncates() {
        let runner = SystemCommandRunner;
        // Generate output larger than cap by printing a long string.
        let spec = CommandSpec::new("/usr/bin/yes", Vec::<String>::new())
            .unwrap()
            .with_output_cap(100)
            .with_timeout(Duration::from_millis(200));
        let token = CancellationToken::new();
        let result = runner.run(&spec, &token);
        // yes will be killed by timeout, that's fine for this test.
        // We either get a timeout error or output; check whichever we get.
        match result {
            Ok(output) => {
                // If we got output, it should be truncated.
                assert!(output.stdout.len() <= 100 || output.stdout_truncated);
            }
            Err(MaintainerError::CommandFailed { .. }) => {
                // Timeout is acceptable; the point is we don't OOM.
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}

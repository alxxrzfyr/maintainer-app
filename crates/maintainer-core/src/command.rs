use std::collections::HashSet;
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
        "nproc",
        "xdg-open",
        "gio",
    ]
    .into_iter()
    .collect()
});

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
    use std::io::{BufRead, BufReader, Read};
    use std::sync::mpsc;

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
    let output_cap = spec.output_cap_bytes();

    // Collect stderr in a background thread to prevent deadlock.
    let stderr_pipe = child.stderr.take().expect("stderr was piped");
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut reader = stderr_pipe;
        let _ = reader.read_to_end(&mut buf);
        buf
    });

    // Read stdout lines in a background thread, sending them through a channel.
    let stdout_pipe = child.stdout.take().expect("stdout was piped");
    let (line_tx, line_rx) = mpsc::sync_channel::<String>(64);
    let stdout_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stdout_pipe);
        let mut all_bytes = Vec::new();
        for line_result in reader.lines() {
            match line_result {
                Ok(line) => {
                    // Accumulate raw bytes for final output
                    if all_bytes.len() < output_cap {
                        let remaining = output_cap - all_bytes.len();
                        let line_bytes = line.as_bytes();
                        let to_copy = (line_bytes.len() + 1).min(remaining); // +1 for newline
                        if to_copy > 0 {
                            let copy_line = to_copy.min(line_bytes.len());
                            all_bytes.extend_from_slice(&line_bytes[..copy_line]);
                            if to_copy > line_bytes.len() {
                                all_bytes.push(b'\n');
                            }
                        }
                    }
                    // Send line to the main thread; if receiver is gone, stop.
                    if line_tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        all_bytes
    });

    // Poll the channel, delivering lines to the callback while checking
    // cancellation and timeout.
    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            // Drop receiver so stdout thread exits
            drop(line_rx);
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            return Err(MaintainerError::Cancelled);
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            drop(line_rx);
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            return Err(MaintainerError::CommandFailed {
                message: format!("{} timed out after {:?}", spec.program(), spec.timeout()),
            });
        }

        match line_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                on_line(&line);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Check if child has exited (stdout thread will have finished)
                // by seeing if the channel is disconnected on next iteration.
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // Stdout thread finished (EOF or error). Break to collect results.
                break;
            }
        }
    }

    // Wait for the child to exit.
    let status = child
        .wait()
        .map_err(|e| MaintainerError::io(format!("waiting for {}", spec.program()), e))?;

    // Collect outputs from background threads.
    let stdout_bytes = stdout_handle.join().unwrap_or_default();
    let stderr_raw = stderr_handle.join().unwrap_or_default();

    let stdout_truncated = stdout_bytes.len() >= output_cap;
    let stdout = String::from_utf8_lossy(&stdout_bytes).into_owned();
    let (stderr, stderr_truncated) = truncate_output(&stderr_raw, output_cap);

    Ok(CommandOutput {
        exit_code: status.code(),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
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

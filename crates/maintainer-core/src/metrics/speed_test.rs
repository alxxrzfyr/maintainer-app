//! Internet speed test service using Cloudflare's fixed endpoint.
//!
//! Runs curl against `speed.cloudflare.com` with a finite upload payload.
//! Supports cancellation (terminates child process) and reports typed results.
//!
//! Provider: Cloudflare (speed.cloudflare.com)
//! Endpoints:
//!   - Latency: `https://speed.cloudflare.com/__down?bytes=0`
//!   - Download: `https://speed.cloudflare.com/__down?bytes=10000000`
//!   - Upload: `https://speed.cloudflare.com/__up`
//!
//! Data usage: ~10–25 MB per full test run.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::MaintainerError;

/// Cloudflare latency endpoint (zero-byte download for TTFB measurement).
pub const LATENCY_URL: &str = "https://speed.cloudflare.com/__down?bytes=0";

/// Cloudflare download endpoint (10 MB payload).
pub const DOWNLOAD_URL: &str = "https://speed.cloudflare.com/__down?bytes=10000000";

/// Cloudflare upload endpoint.
pub const UPLOAD_URL: &str = "https://speed.cloudflare.com/__up";

/// Size of the finite upload payload in bytes (5 MB).
pub const UPLOAD_SIZE_BYTES: usize = 5_000_000;

/// Fixed absolute path to curl, if it exists at the standard location.
const CURL_ABSOLUTE: &str = "/usr/bin/curl";

/// Result of a completed speed test.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeedTestResult {
    /// Download speed in megabits per second.
    pub download_mbps: f64,
    /// Upload speed in megabits per second.
    pub upload_mbps: f64,
    /// Latency (time to first byte) in milliseconds.
    pub latency_ms: f64,
}

/// Current phase of the speed test for progress reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedTestStage {
    /// Measuring latency (TTFB).
    Latency,
    /// Running download measurement.
    Download,
    /// Running upload measurement.
    Upload,
}

impl SpeedTestStage {
    /// Human-readable label for this stage.
    pub fn label(self) -> &'static str {
        match self {
            Self::Latency => "Measuring latency",
            Self::Download => "Testing download speed",
            Self::Upload => "Testing upload speed",
        }
    }
}

/// Resolve the curl binary path: prefer absolute path, fall back to PATH lookup.
pub fn resolve_curl_path() -> Result<PathBuf, String> {
    let absolute = Path::new(CURL_ABSOLUTE);
    if absolute.is_file() {
        return Ok(absolute.to_path_buf());
    }
    // Fall back to searching PATH via `which`.
    match which_curl() {
        Some(path) => Ok(path),
        None => Err("curl is not installed (checked /usr/bin/curl and PATH)".to_string()),
    }
}

/// Check if curl binary is available.
pub fn check_curl_available() -> Result<PathBuf, String> {
    resolve_curl_path()
}

/// Build the argument array for the latency measurement.
pub fn build_latency_args() -> Vec<String> {
    vec![
        "--silent".to_string(),
        "--fail".to_string(),
        "--output".to_string(),
        "/dev/null".to_string(),
        "--max-time".to_string(),
        "10".to_string(),
        "--write-out".to_string(),
        "%{time_starttransfer}".to_string(),
        LATENCY_URL.to_string(),
    ]
}

/// Build the argument array for the download measurement.
pub fn build_download_args() -> Vec<String> {
    vec![
        "--silent".to_string(),
        "--fail".to_string(),
        "--output".to_string(),
        "/dev/null".to_string(),
        "--max-time".to_string(),
        "30".to_string(),
        "--write-out".to_string(),
        "%{speed_download}".to_string(),
        DOWNLOAD_URL.to_string(),
    ]
}

/// Build the argument array for the upload measurement.
///
/// Uses a finite file (never `/dev/zero`). Does not set a manual
/// `Content-Length` header — curl computes it from the file size.
pub fn build_upload_args(payload_path: &Path) -> Vec<String> {
    let data_arg = format!("@{}", payload_path.display());
    vec![
        "--silent".to_string(),
        "--fail".to_string(),
        "--output".to_string(),
        "/dev/null".to_string(),
        "--max-time".to_string(),
        "30".to_string(),
        "--write-out".to_string(),
        "%{speed_upload}".to_string(),
        "--data-binary".to_string(),
        data_arg,
        UPLOAD_URL.to_string(),
    ]
}

/// Create a finite upload payload file of exactly `UPLOAD_SIZE_BYTES`.
pub fn create_upload_payload(dir: &Path) -> Result<PathBuf, MaintainerError> {
    let path = dir.join("maintainer-speedtest-payload.bin");
    let mut f = std::fs::File::create(&path).map_err(|e| {
        MaintainerError::io(format!("creating upload payload at {}", path.display()), e)
    })?;
    f.write_all(&vec![0u8; UPLOAD_SIZE_BYTES]).map_err(|e| {
        MaintainerError::io(format!("writing upload payload at {}", path.display()), e)
    })?;
    Ok(path)
}

/// Parse curl's `%{speed_download}` or `%{speed_upload}` output (bytes/sec)
/// into megabits per second. Returns `None` if the value is not a finite
/// positive number.
pub fn parse_speed_bps_to_mbps(raw: &str) -> Option<f64> {
    let bps: f64 = raw.trim().parse().ok()?;
    if !bps.is_finite() || bps <= 0.0 {
        return None;
    }
    Some((bps * 8.0) / 1_000_000.0)
}

/// Parse curl's `%{time_starttransfer}` output (seconds) into milliseconds.
/// Returns `None` if the value is not a finite positive number.
pub fn parse_latency_secs_to_ms(raw: &str) -> Option<f64> {
    let secs: f64 = raw.trim().parse().ok()?;
    if !secs.is_finite() || secs <= 0.0 {
        return None;
    }
    Some(secs * 1000.0)
}

/// Run the full speed test. Caller provides a cancellation flag.
///
/// Progress is reported via the `on_stage` callback, which is invoked at the
/// start of each measurement phase.
///
/// Cancellation terminates the child process (kills the curl subprocess)
/// rather than merely ignoring the eventual result.
pub fn run_speed_test(
    cancel: &Arc<AtomicBool>,
    on_stage: impl Fn(SpeedTestStage),
) -> Result<SpeedTestResult, MaintainerError> {
    let curl_path =
        resolve_curl_path().map_err(|msg| MaintainerError::CommandFailed { message: msg })?;

    if cancel.load(Ordering::SeqCst) {
        return Err(MaintainerError::Cancelled);
    }

    // --- Latency ---
    on_stage(SpeedTestStage::Latency);
    let latency_ms = {
        let args = build_latency_args();
        let output = run_curl_cancellable(&curl_path, &args, cancel)?;
        parse_latency_secs_to_ms(&output).ok_or_else(|| MaintainerError::InvalidData {
            source_name: "speed-test-latency",
            message: format!("could not parse latency from curl output: '{output}'"),
        })?
    };

    if cancel.load(Ordering::SeqCst) {
        return Err(MaintainerError::Cancelled);
    }

    // --- Download ---
    on_stage(SpeedTestStage::Download);
    let download_mbps = {
        let args = build_download_args();
        let output = run_curl_cancellable(&curl_path, &args, cancel)?;
        parse_speed_bps_to_mbps(&output).ok_or_else(|| MaintainerError::InvalidData {
            source_name: "speed-test-download",
            message: format!("could not parse download speed from curl output: '{output}'"),
        })?
    };

    if cancel.load(Ordering::SeqCst) {
        return Err(MaintainerError::Cancelled);
    }

    // --- Upload ---
    on_stage(SpeedTestStage::Upload);
    let upload_mbps = {
        let temp_dir = std::env::temp_dir();
        let payload_path = create_upload_payload(&temp_dir)?;
        let args = build_upload_args(&payload_path);
        let result = run_curl_cancellable(&curl_path, &args, cancel);

        // Clean up payload regardless of result.
        let _ = std::fs::remove_file(&payload_path);

        let output = result?;
        parse_speed_bps_to_mbps(&output).ok_or_else(|| MaintainerError::InvalidData {
            source_name: "speed-test-upload",
            message: format!("could not parse upload speed from curl output: '{output}'"),
        })?
    };

    Ok(SpeedTestResult {
        download_mbps,
        upload_mbps,
        latency_ms,
    })
}

/// Execute curl with the given arguments. Supports cancellation by killing the
/// child process. With `--fail` in the argument array, curl returns exit code 22
/// on HTTP 4xx/5xx responses, which `status.success()` correctly rejects.
fn run_curl_cancellable(
    curl_path: &Path,
    args: &[String],
    cancel: &Arc<AtomicBool>,
) -> Result<String, MaintainerError> {
    use std::process::{Command, Stdio};

    let mut child = Command::new(curl_path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| MaintainerError::CommandFailed {
            message: format!("failed to start curl: {e}"),
        })?;

    // Poll for completion or cancellation.
    loop {
        if cancel.load(Ordering::SeqCst) {
            // Kill the child process — do not merely ignore the result.
            let _ = child.kill();
            let _ = child.wait();
            return Err(MaintainerError::Cancelled);
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(MaintainerError::CommandFailed {
                        message: format!(
                            "curl exited with status {} (non-2xx or network error)",
                            status.code().unwrap_or(-1)
                        ),
                    });
                }
                break;
            }
            Ok(None) => {
                // Child still running — brief sleep to avoid busy-wait.
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                return Err(MaintainerError::CommandFailed {
                    message: format!("error waiting for curl: {e}"),
                });
            }
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| MaintainerError::CommandFailed {
            message: format!("failed to read curl output: {e}"),
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(stdout)
}

/// Try to find curl in PATH (fallback if /usr/bin/curl doesn't exist).
fn which_curl() -> Option<PathBuf> {
    let output = std::process::Command::new("which")
        .arg("curl")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path_str.is_empty() {
        return None;
    }
    let p = PathBuf::from(&path_str);
    if p.is_file() { Some(p) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_payload_is_finite_and_correct_size() {
        let dir = std::env::temp_dir();
        let path = create_upload_payload(&dir).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), UPLOAD_SIZE_BYTES as u64);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upload_args_use_finite_file_not_dev_zero() {
        let args = build_upload_args(Path::new("/tmp/payload.bin"));
        assert!(
            !args.iter().any(|a| a.contains("/dev/zero")),
            "upload must never reference /dev/zero"
        );
        assert!(
            args.iter().any(|a| a.contains("@/tmp/payload.bin")),
            "upload must reference the finite payload file"
        );
    }

    #[test]
    fn upload_args_do_not_set_manual_content_length() {
        let args = build_upload_args(Path::new("/tmp/payload.bin"));
        assert!(
            !args.iter().any(|a| a.contains("Content-Length")),
            "upload must not set a manual Content-Length header"
        );
    }

    #[test]
    fn upload_args_do_not_use_explicit_request_post() {
        let args = build_upload_args(Path::new("/tmp/payload.bin"));
        assert!(
            !args.iter().any(|a| a == "--request" || a == "POST"),
            "--data-binary implies POST; explicit --request POST is redundant"
        );
    }

    #[test]
    fn latency_args_are_well_formed() {
        let args = build_latency_args();
        assert!(args.contains(&"--silent".to_string()));
        assert!(args.contains(&"%{time_starttransfer}".to_string()));
        assert!(args.contains(&LATENCY_URL.to_string()));
    }

    #[test]
    fn download_args_are_well_formed() {
        let args = build_download_args();
        assert!(args.contains(&"--silent".to_string()));
        assert!(args.contains(&"%{speed_download}".to_string()));
        assert!(args.contains(&DOWNLOAD_URL.to_string()));
    }

    #[test]
    fn all_curl_args_include_fail_flag_for_http_error_detection() {
        let latency_args = build_latency_args();
        assert!(
            latency_args.contains(&"--fail".to_string()),
            "latency args must include --fail to reject HTTP 4xx/5xx"
        );

        let download_args = build_download_args();
        assert!(
            download_args.contains(&"--fail".to_string()),
            "download args must include --fail to reject HTTP 4xx/5xx"
        );

        let upload_args = build_upload_args(Path::new("/tmp/payload.bin"));
        assert!(
            upload_args.contains(&"--fail".to_string()),
            "upload args must include --fail to reject HTTP 4xx/5xx"
        );
    }

    #[test]
    fn parse_speed_bps_valid_float() {
        // 1234567.890 bytes/sec → ~9.876 Mbps
        let mbps = parse_speed_bps_to_mbps("1234567.890").unwrap();
        assert!((mbps - 9.876).abs() < 0.01);
    }

    #[test]
    fn parse_speed_bps_zero_returns_none() {
        assert_eq!(parse_speed_bps_to_mbps("0.000"), None);
        assert_eq!(parse_speed_bps_to_mbps("0"), None);
    }

    #[test]
    fn parse_speed_bps_negative_returns_none() {
        assert_eq!(parse_speed_bps_to_mbps("-100.0"), None);
    }

    #[test]
    fn parse_speed_bps_nan_returns_none() {
        assert_eq!(parse_speed_bps_to_mbps("NaN"), None);
        assert_eq!(parse_speed_bps_to_mbps("inf"), None);
    }

    #[test]
    fn parse_speed_bps_garbage_returns_none() {
        assert_eq!(parse_speed_bps_to_mbps("not_a_number"), None);
        assert_eq!(parse_speed_bps_to_mbps(""), None);
    }

    #[test]
    fn parse_latency_valid() {
        let ms = parse_latency_secs_to_ms("0.042").unwrap();
        assert!((ms - 42.0).abs() < 0.001);
    }

    #[test]
    fn parse_latency_zero_returns_none() {
        assert_eq!(parse_latency_secs_to_ms("0.000"), None);
    }

    #[test]
    fn parse_latency_garbage_returns_none() {
        assert_eq!(parse_latency_secs_to_ms("abc"), None);
    }

    #[test]
    fn cancellation_before_start_returns_cancelled() {
        let cancel = Arc::new(AtomicBool::new(true));
        let result = run_speed_test(&cancel, |_| {});
        match result {
            Err(MaintainerError::Cancelled) => {} // expected
            other => panic!("expected Cancelled, got {other:?}"),
        }
    }

    #[test]
    fn stage_labels_are_descriptive() {
        assert_eq!(SpeedTestStage::Latency.label(), "Measuring latency");
        assert_eq!(SpeedTestStage::Download.label(), "Testing download speed");
        assert_eq!(SpeedTestStage::Upload.label(), "Testing upload speed");
    }

    #[test]
    fn curl_path_uses_absolute_when_available() {
        // This test verifies the constant is an absolute path.
        assert!(CURL_ABSOLUTE.starts_with('/'));
    }
}

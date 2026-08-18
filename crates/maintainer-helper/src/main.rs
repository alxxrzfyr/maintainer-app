//! Maintainer privileged helper entry point.
//!
//! Launched by D-Bus activation only. Exits after idle timeout.
//! Not a permanent daemon.

use maintainer_helper::api::IDLE_TIMEOUT_SECS;

fn main() {
    // The helper is designed to be D-Bus activated. When launched without
    // a D-Bus activation environment, it prints usage and exits.
    //
    // Full D-Bus server integration will be connected when the zbus
    // async runtime is wired up. For now, the binary validates that all
    // modules compile and the idle timeout is configured.
    eprintln!("maintainer-helper: D-Bus activated helper (idle timeout: {IDLE_TIMEOUT_SECS}s)");
    eprintln!("maintainer-helper: not intended to be run directly; activate via D-Bus");
    std::process::exit(1);
}

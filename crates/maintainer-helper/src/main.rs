//! Maintainer privileged helper entry point.
//!
//! Launched by D-Bus activation only. Exits after idle timeout.
//! Not a permanent daemon.

use std::sync::Arc;

use maintainer_helper::authorize::SystemPolicy;
use maintainer_helper::server::{ServerConfig, run_server};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Provide a minimal --help for discoverability.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return;
    }

    let policy = Arc::new(SystemPolicy);
    let config = ServerConfig::default();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| {
            eprintln!("maintainer-helper: failed to create runtime: {e}");
            std::process::exit(1);
        });

    eprintln!(
        "maintainer-helper: starting on system bus as {}",
        config.bus_name
    );

    if let Err(e) = rt.block_on(run_server(config, policy)) {
        eprintln!("maintainer-helper: {e}");
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!(
        "maintainer-helper — D-Bus activated privileged helper for Maintainer\n\
         \n\
         Usage: maintainer-helper [OPTIONS]\n\
         \n\
         This binary is not intended to be run directly. It is activated by\n\
         D-Bus when the Maintainer application requests a privileged operation.\n\
         \n\
         Options:\n\
         \x20 --help, -h  Show this message\n\
         \n\
         Idle timeout: {}s\n\
         Bus name: io.github.maintainerapp.Helper\n\
         Object path: /io/github/maintainerapp/Helper",
        maintainer_helper::api::IDLE_TIMEOUT_SECS
    );
}

//! Maintainer privileged helper entry point.
//!
//! Launched by D-Bus activation only. Exits after idle timeout.
//! Not a permanent daemon.

use std::sync::Arc;

use maintainer_helper::authorize::SystemPolicy;
use maintainer_helper::server::{ServerConfig, run_server};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return;
    }

    // pkexec sets PKEXEC_UID for the invoking user. Under that trust boundary,
    // this binary accepts exactly one fixed operation; it never starts the
    // general D-Bus service or accepts a caller-provided program/option list.
    if std::env::var_os("PKEXEC_UID").is_some() {
        if args.first().map(String::as_str) != Some("install-updates") {
            eprintln!("maintainer-helper: pkexec invocation requires install-updates");
            std::process::exit(2);
        }
        let request = maintainer_helper::validate::parse_install_updates_cli(&args[1..])
            .unwrap_or_else(|error| {
                eprintln!("maintainer-helper: {error}");
                std::process::exit(2);
            });
        let error = maintainer_helper::operations::updates::exec_install_updates(&request);
        eprintln!("maintainer-helper: failed to execute apt-get: {error}");
        std::process::exit(1);
    }

    if !args.is_empty() {
        eprintln!("maintainer-helper: unsupported arguments");
        std::process::exit(2);
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| {
            eprintln!("maintainer-helper: failed to create runtime: {e}");
            std::process::exit(1);
        });

    // Connect to system bus for credential resolution and Polkit checks.
    let bus_connection = zbus::blocking::Connection::system().unwrap_or_else(|e| {
        eprintln!("maintainer-helper: failed to connect to system bus: {e}");
        std::process::exit(1);
    });

    let policy = Arc::new(SystemPolicy::new(bus_connection));
    let config = ServerConfig::default();

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

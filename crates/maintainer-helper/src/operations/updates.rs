//! Refuse package execution until an authoritative reviewed-plan owner exists.

use crate::api::InstallUpdatesRequest;

/// The legacy CLI cannot bind the complete dependency plan to execution or
/// retain ownership across GUI disconnects. Version pins cover only requested
/// packages. Neither repeating simulation nor a separate PackageKit simulation
/// freezes the resolver state used by the eventual transaction.
///
/// Keep this entrypoint fail-closed so direct invocations cannot bypass the GUI
/// capability check. No child process or package-manager lock is acquired.
pub fn exec_install_updates(_request: &InstallUpdatesRequest) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        maintainer_core::updates::PACKAGE_EXECUTION_UNAVAILABLE_REASON,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_upgrade_cannot_enter_unbound_package_execution() {
        let error = exec_install_updates(&InstallUpdatesRequest::Full);
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("reviewed-plan binding"));
    }

    #[test]
    fn version_pins_cannot_enable_unbound_dependency_execution() {
        let request = InstallUpdatesRequest::Selected(vec![
            "curl=8.5.0-2ubuntu10.6".to_string(),
            "libc6:amd64=2.39-0ubuntu8.4".to_string(),
        ]);
        let error = exec_install_updates(&request);
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("transaction ownership"));
    }
}

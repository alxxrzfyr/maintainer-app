//! Blocking package-owner transport. Create connections only inside workers.

use std::sync::{Arc, Mutex};

use maintainer_core::package_transaction::{PackageReceipt, valid_transaction_id};

const BUS: &str = "io.github.maintainerapp.PackageOwner";
const PATH: &str = "/io/github/maintainerapp/PackageOwner";

#[derive(Clone, Default)]
pub struct PackageClient {
    connection: Arc<Mutex<Option<zbus::blocking::Connection>>>,
}

impl PackageClient {
    fn with_proxy<T>(
        &self,
        call: impl FnOnce(&zbus::blocking::Proxy<'_>) -> Result<T, zbus::Error>,
    ) -> Result<T, String> {
        let mut retained = self
            .connection
            .lock()
            .map_err(|_| "Package connection unavailable.")?;
        if retained.is_none() {
            *retained = Some(zbus::blocking::Connection::system().map_err(|_| {
                "Package service unavailable. Install the Maintainer package and system service."
            })?);
        }
        let connection = retained.as_ref().ok_or("Package connection unavailable.")?;
        let proxy = zbus::blocking::Proxy::new(connection, BUS, PATH, BUS)
            .map_err(|_| "Package service unavailable.")?;
        // Never replace a disconnected connection during an operation: its bus
        // identity belongs to the authorization and retained control channel.
        call(&proxy).map_err(|error| match error {
            zbus::Error::MethodError(name, _, _) if name.as_str().contains("AccessDenied") =>
                "Administrator permission was denied or dismissed.".into(),
            zbus::Error::MethodError(name, _, _) if name.as_str().contains("ServiceUnknown") =>
                "Package service unavailable. Install the Maintainer system service.".into(),
            _ => "Package service could not confirm the outcome. Use Resume Package Operation; do not start it again.".into(),
        })
    }

    pub fn begin(&self, intent: &str, requests: &[String], id: &str) -> Result<String, String> {
        self.with_proxy(|proxy| proxy.call("Begin", &(intent, requests, id)))
    }

    pub fn status(&self, id: &str) -> Result<(PackageReceipt, bool), String> {
        let (text, cancellable): (String, bool) =
            self.with_proxy(|proxy| proxy.call("Status", &(id,)))?;
        let receipt = PackageReceipt::parse(&text)
            .map_err(|_| "Package service returned an invalid receipt.".to_string())?;
        if receipt.id != id {
            return Err("Package service returned a different transaction.".into());
        }
        Ok((receipt, cancellable))
    }

    pub fn approve(&self, id: &str, fingerprint: &str) -> Result<(), String> {
        self.with_proxy(|proxy| proxy.call("Approve", &(id, fingerprint)))
    }

    pub fn cancel(&self, id: &str) -> Result<bool, String> {
        self.with_proxy(|proxy| proxy.call("Cancel", &(id,)))
    }

    pub fn reconcile(&self, id: &str) -> Result<(), String> {
        self.with_proxy(|proxy| proxy.call("Reconcile", &(id,)))
    }

    pub fn recent(&self) -> Result<Vec<String>, String> {
        let ids: Vec<String> = self.with_proxy(|proxy| proxy.call("Recent", &()))?;
        if ids.len() > 32 || ids.iter().any(|id| !valid_transaction_id(id)) {
            return Err("Package service returned invalid recent transactions.".into());
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Owner {
        senders: Arc<Mutex<Vec<String>>>,
    }

    #[zbus::interface(name = "io.github.maintainerapp.PackageOwner")]
    impl Owner {
        fn recent(&self, #[zbus(header)] header: zbus::message::Header<'_>) -> Vec<String> {
            self.senders
                .lock()
                .unwrap()
                .push(header.sender().unwrap().to_string());
            vec!["a".repeat(32)]
        }

        fn status(&self, _id: &str) -> (String, bool) {
            (
                format!(
                    "VERSION\t1\nID\t{}\nUID\t1000\nPID\t12\nSTART\t1\nINTENT\tfull\nSTATE\treview\nPLAN\t{}\t0\nEND\n",
                    "b".repeat(32),
                    "c".repeat(64)
                ),
                true,
            )
        }
    }

    #[test]
    fn private_bus_keeps_control_identity_and_rejects_mismatched_receipts() {
        let senders = Arc::new(Mutex::new(Vec::new()));
        let _server = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name(BUS)
            .unwrap()
            .serve_at(
                PATH,
                Owner {
                    senders: senders.clone(),
                },
            )
            .unwrap()
            .build()
            .unwrap();
        let client = PackageClient {
            connection: Arc::new(Mutex::new(Some(
                zbus::blocking::Connection::session().unwrap(),
            ))),
        };
        assert_eq!(client.recent().unwrap(), vec!["a".repeat(32)]);
        assert_eq!(client.recent().unwrap(), vec!["a".repeat(32)]);
        let observed = senders.lock().unwrap();
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0], observed[1]);
        assert!(
            client
                .status(&"a".repeat(32))
                .unwrap_err()
                .contains("different transaction")
        );
    }
}

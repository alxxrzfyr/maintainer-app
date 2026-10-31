//! Authorized cleanup recovery; never replaces a destination entry.
use std::fs::OpenOptions;
use std::os::fd::AsFd;
use std::os::unix::fs::OpenOptionsExt;

use crate::cleanup::quarantine::Recovery;

pub fn list() -> Result<Vec<Recovery>, String> {
    let connection = zbus::blocking::Connection::system().map_err(|error| error.to_string())?;
    let reply = connection
        .call_method(
            Some("io.github.maintainerapp.Helper"),
            "/io/github/maintainerapp/Helper",
            Some("io.github.maintainerapp.Helper"),
            "ListCleanupRecovery",
            &(),
        )
        .map_err(|error| error.to_string())?;
    let rows: Vec<(String, String, String)> = reply
        .body()
        .deserialize()
        .map_err(|error| error.to_string())?;
    if rows.len() > 128 {
        return Err("Recovery response exceeds the bounded item limit".into());
    }
    Ok(rows
        .into_iter()
        .map(|(id, basename, parent_path)| Recovery {
            id,
            basename,
            parent_path,
        })
        .collect())
}

pub fn restore(
    item: &Recovery,
    elsewhere: Option<&std::path::Path>,
) -> Result<Option<String>, String> {
    let parent = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(elsewhere.unwrap_or_else(|| std::path::Path::new(&item.parent_path)))
        .map_err(|error| {
            format!("Original parent unavailable; retained file remains safe: {error}")
        })?;
    let connection = zbus::blocking::Connection::system().map_err(|error| error.to_string())?;
    let reply = connection
        .call_method(
            Some("io.github.maintainerapp.Helper"),
            "/io/github/maintainerapp/Helper",
            Some("io.github.maintainerapp.Helper"),
            if elsewhere.is_some() {
                "RestoreCleanupElsewhere"
            } else {
                "RestoreCleanupRecovery"
            },
            &(item.id.as_str(), zbus::zvariant::Fd::from(parent.as_fd())),
        )
        .map_err(|error| error.to_string())?;
    let (success, message): (bool, String) = reply
        .body()
        .deserialize()
        .map_err(|error| error.to_string())?;
    if success {
        Ok((!message.is_empty()).then_some(message))
    } else {
        Err(message)
    }
}

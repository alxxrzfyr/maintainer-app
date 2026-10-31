//! Authorized descriptor capabilities for caller-owned reviewed regular files.
use std::fs::File;
use std::os::fd::OwnedFd;
use std::sync::Mutex;

use crate::api::HelperError;
use maintainer_core::cleanup::quarantine::{Quarantine, Removal, Restoration};

// A single owner bounds receipts and keeps recovery and removal from racing.
static TRANSACTION: Mutex<()> = Mutex::new(());

fn failure(error: std::io::Error) -> HelperError {
    HelperError::OperationFailed {
        reason: error.to_string(),
    }
}

pub fn remove(
    uid: u32,
    parent: OwnedFd,
    reviewed: OwnedFd,
    basename: &str,
    trash: bool,
    files: OwnedFd,
    info: OwnedFd,
) -> Result<(bool, String), HelperError> {
    let _guard = TRANSACTION
        .lock()
        .map_err(|_| HelperError::OperationFailed {
            reason: "cleanup transaction lock unavailable".into(),
        })?;
    let store = Quarantine::system().map_err(failure)?;
    let parent = File::from(parent);
    let reviewed = File::from(reviewed);
    let result = if trash {
        store.trash(
            uid,
            &parent,
            &reviewed,
            basename,
            &File::from(files),
            &File::from(info),
        )
    } else {
        store.remove(uid, &parent, &reviewed, basename, || false)
    };
    match result.map_err(failure)? {
        Removal::Trashed => Ok((true, String::new())),
        Removal::Deleted => Ok((true, String::new())),
        Removal::TrashedDurabilityUnknown => Ok((true, "File is in Trash, but durability or receipt cleanup is unconfirmed. Do not retry removal.".into())),
        Removal::DeletedDurabilityUnknown => Ok((true, "File was permanently removed, but durability or receipt cleanup is unconfirmed. Do not retry removal.".into())),
        Removal::RestoredDurabilityUnknown => Ok((false, "Entry was restored without deletion, but durability or receipt cleanup is unconfirmed. Do not retry restoration.".into())),
        Removal::Restored => Ok((
            false,
            "File changed or operation cancelled; entry restored without deletion".into(),
        )),
        Removal::RecoveryRequired(id) => Ok((
            false,
            format!(
                "Recovery required: {id}. The retained entry was not deleted. Restore it from Cleanup recovery."
            ),
        )),
    }
}

pub fn list(uid: u32) -> Result<Vec<(String, String, String)>, HelperError> {
    let _guard = TRANSACTION
        .lock()
        .map_err(|_| HelperError::OperationFailed {
            reason: "cleanup transaction lock unavailable".into(),
        })?;
    Ok(Quarantine::system()
        .map_err(failure)?
        .list(uid)
        .map_err(failure)?
        .into_iter()
        .map(|item| (item.id, item.basename, item.parent_path))
        .collect())
}

pub fn restore(
    uid: u32,
    id: &str,
    parent: OwnedFd,
    elsewhere: bool,
) -> Result<(bool, String), HelperError> {
    let _guard = TRANSACTION
        .lock()
        .map_err(|_| HelperError::OperationFailed {
            reason: "cleanup transaction lock unavailable".into(),
        })?;
    let store = Quarantine::system().map_err(failure)?;
    let parent = File::from(parent);
    let result = if elsewhere {
        store.restore_elsewhere(uid, id, &parent)
    } else {
        store.restore(uid, id, &parent)
    }
    .map_err(failure)?;
    Ok((true, match result {
        Restoration::Restored => String::new(),
        Restoration::RestoredDurabilityUnknown => "Entry was restored without overwriting another file, but durability or receipt cleanup is unconfirmed. Do not retry restoration.".into(),
    }))
}

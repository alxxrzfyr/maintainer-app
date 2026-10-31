//! One authorization grants only a bounded reviewed manifest on its originating
//! live D-Bus connection. A consumed index cannot be retried or transferred.
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::time::{Duration, Instant};

use crate::api::HelperError;
pub type ManifestEntry = (String, u64, u64, u64, u64, bool);

struct Grant {
    id: String,
    sender: String,
    uid: u32,
    expires: Instant,
    manifest: Vec<ManifestEntry>,
    consumed: Vec<bool>,
}

#[derive(Default)]
pub(crate) struct CleanupBatches(Vec<Grant>);
fn invalid(reason: &str) -> HelperError {
    HelperError::ValidationFailed {
        reason: reason.into(),
    }
}

impl CleanupBatches {
    pub fn validate(&mut self, manifest: &[ManifestEntry]) -> Result<(), HelperError> {
        self.0.retain(|grant| grant.expires > Instant::now());
        if self.0.len() >= 4 {
            return Err(HelperError::Busy);
        }
        if manifest.is_empty() || manifest.len() > 256 {
            return Err(invalid(
                "cleanup batch must contain 1 to 256 reviewed files",
            ));
        }
        for (name, _, parent_inode, _, file_inode, _) in manifest {
            maintainer_core::cleanup::quarantine::component(name)
                .map_err(|_| invalid("invalid reviewed filename"))?;
            if *parent_inode == 0 || *file_inode == 0 {
                return Err(invalid("missing reviewed identity"));
            }
        }
        Ok(())
    }

    pub fn issue(
        &mut self,
        sender: &str,
        uid: u32,
        manifest: Vec<ManifestEntry>,
    ) -> Result<String, HelperError> {
        let mut random = [0u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut random))
            .map_err(|_| invalid("cannot create cleanup grant"))?;
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let id: String = random
            .iter()
            .flat_map(|byte| {
                [
                    char::from(HEX[(byte >> 4) as usize]),
                    char::from(HEX[(byte & 15) as usize]),
                ]
            })
            .collect();
        let consumed = vec![false; manifest.len()];
        self.0.push(Grant {
            id: id.clone(),
            sender: sender.into(),
            uid,
            expires: Instant::now() + Duration::from_secs(300),
            manifest,
            consumed,
        });
        Ok(id)
    }

    fn find(&mut self, sender: &str, uid: u32, id: &str) -> Result<&mut Grant, HelperError> {
        self.0.retain(|grant| grant.expires > Instant::now());
        self.0
            .iter_mut()
            .find(|grant| grant.id == id && grant.sender == sender && grant.uid == uid)
            .ok_or_else(|| {
                invalid(
                    "cleanup grant lost, expired or belongs to another connection; review again",
                )
            })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "The approved entry binds both descriptor identities and the fixed removal mode"
    )]
    pub fn consume(
        &mut self,
        sender: &str,
        uid: u32,
        id: &str,
        index: u32,
        parent: &File,
        file: &File,
        name: &str,
        trash: bool,
    ) -> Result<(), HelperError> {
        let grant = self.find(sender, uid, id)?;
        let index = index as usize;
        let expected = grant
            .manifest
            .get(index)
            .ok_or_else(|| invalid("cleanup batch index out of range"))?;
        let parent = parent
            .metadata()
            .map_err(|_| invalid("invalid parent descriptor"))?;
        let file = file
            .metadata()
            .map_err(|_| invalid("invalid reviewed descriptor"))?;
        let actual = (
            name.to_string(),
            parent.dev(),
            parent.ino(),
            file.dev(),
            file.ino(),
            trash,
        );
        if grant.consumed[index]
            || expected != &actual
            || parent.uid() != uid
            || file.uid() != uid
            || !parent.is_dir()
            || !file.is_file()
        {
            return Err(invalid(
                "cleanup entry replay or manifest/descriptor mismatch",
            ));
        }
        grant.consumed[index] = true;
        Ok(())
    }

    pub fn end(&mut self, sender: &str, uid: u32, id: &str) -> Result<(), HelperError> {
        self.find(sender, uid, id)?;
        self.0.retain(|grant| grant.id != id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> (File, File, ManifestEntry, u32) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let parent = File::open(root).unwrap();
        let file = File::open(root.join("Cargo.toml")).unwrap();
        let p = parent.metadata().unwrap();
        let f = file.metadata().unwrap();
        let entry = (
            "Cargo.toml".into(),
            p.dev(),
            p.ino(),
            f.dev(),
            f.ino(),
            false,
        );
        (parent, file, entry, f.uid())
    }
    #[test]
    fn grant_binds_uid_bus_manifest_mode_index_and_rejects_replay() {
        let (parent, file, entry, uid) = target();
        let mut batches = CleanupBatches::default();
        batches.validate(&[entry.clone(), entry.clone()]).unwrap();
        let id = batches
            .issue(":1.10", uid, vec![entry.clone(), entry])
            .unwrap();
        assert!(
            batches
                .consume(":1.11", uid, &id, 0, &parent, &file, "Cargo.toml", false)
                .is_err()
        );
        assert!(
            batches
                .consume(
                    ":1.10",
                    uid.wrapping_add(1),
                    &id,
                    0,
                    &parent,
                    &file,
                    "Cargo.toml",
                    false
                )
                .is_err()
        );
        assert!(
            batches
                .consume(":1.10", uid, &id, 2, &parent, &file, "Cargo.toml", false)
                .is_err()
        );
        assert!(
            batches
                .consume(":1.10", uid, &id, 0, &parent, &file, "other", false)
                .is_err()
        );
        assert!(
            batches
                .consume(":1.10", uid, &id, 0, &parent, &file, "Cargo.toml", true)
                .is_err()
        );
        assert!(
            batches
                .consume(":1.10", uid, &id, 0, &parent, &parent, "Cargo.toml", false)
                .is_err()
        );
        batches
            .consume(":1.10", uid, &id, 0, &parent, &file, "Cargo.toml", false)
            .unwrap();
        assert!(
            batches
                .consume(":1.10", uid, &id, 0, &parent, &file, "Cargo.toml", false)
                .is_err()
        );
        batches
            .consume(":1.10", uid, &id, 1, &parent, &file, "Cargo.toml", false)
            .unwrap();
        assert!(batches.end(":1.11", uid, &id).is_err());
        batches.end(":1.10", uid, &id).unwrap();
        assert!(batches.end(":1.10", uid, &id).is_err());
    }
    #[test]
    fn expired_grants_are_not_renewed_and_capacity_is_bounded() {
        let (_, _, entry, uid) = target();
        let mut batches = CleanupBatches::default();
        assert!(batches.validate(&[]).is_err());
        assert!(batches.validate(&vec![entry.clone(); 257]).is_err());
        let mut invalid = entry.clone();
        invalid.0 = "../bad".into();
        assert!(batches.validate(&[invalid]).is_err());
        for _ in 0..4 {
            batches.validate(std::slice::from_ref(&entry)).unwrap();
            batches.issue(":1.10", uid, vec![entry.clone()]).unwrap();
        }
        assert!(matches!(
            batches.validate(std::slice::from_ref(&entry)),
            Err(HelperError::Busy)
        ));
        let id = batches.0[0].id.clone();
        batches.0[0].expires = Instant::now() - Duration::from_secs(1);
        assert!(batches.end(":1.10", uid, &id).is_err());
        batches.validate(&[entry]).unwrap();
    }
}

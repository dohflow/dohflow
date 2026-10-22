//! The backup destination seam (personal-cfo-7ie.15): a destination that is not a
//! folder works end to end, and the folder destination behaves as it always has.
//!
//! `MemorySink` keeps its items in a map. It never touches a backup path, so a
//! passing run shows the kernel's backup, verify and restore code reaches a
//! destination only through `BackupSink`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use finance_kernel::{
    BackupHistoryKind, BackupSink, BackupSinkEntry, BackupSinkError, FolderSink, Kernel, PutMode,
};
use tempfile::tempdir;

const PASSWORD: &[u8] = b"correct horse battery";

/// An in-memory destination. `fail_put` and `corrupt_get` model a destination that
/// refuses a write or hands back damaged bytes.
#[derive(Default)]
struct MemorySink {
    items: Mutex<BTreeMap<String, Vec<u8>>>,
    fail_put: bool,
    corrupt_get: bool,
}

impl BackupSink for MemorySink {
    fn describe(&self, name: &str) -> String {
        format!("memory://test/{name}")
    }

    fn put(&self, name: &str, package: &[u8], mode: PutMode) -> Result<u64, BackupSinkError> {
        if self.fail_put {
            return Err(BackupSinkError);
        }
        let mut items = self.items.lock().unwrap();
        if mode == PutMode::CreateNew && items.contains_key(name) {
            return Err(BackupSinkError);
        }
        items.insert(name.to_owned(), package.to_vec());
        Ok(package.len() as u64)
    }

    fn get(&self, name: &str) -> Result<Vec<u8>, BackupSinkError> {
        let mut bytes = self
            .items
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or(BackupSinkError)?;
        if self.corrupt_get {
            let last = bytes.len() - 1;
            bytes[last] ^= 0xff;
        }
        Ok(bytes)
    }

    fn list(&self) -> Result<Vec<BackupSinkEntry>, BackupSinkError> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .iter()
            .map(|(name, bytes)| BackupSinkEntry {
                name: name.clone(),
                size_bytes: bytes.len() as u64,
            })
            .collect())
    }
}

fn kernel() -> (tempfile::TempDir, Kernel) {
    let root = tempdir().unwrap();
    let kernel = Kernel::create_vault(root.path().join("vault.db"), PASSWORD).unwrap();
    (root, kernel)
}

#[test]
fn a_non_folder_destination_writes_verifies_lists_and_restores_a_new_vault() {
    let (_root, kernel) = kernel();
    let sink = MemorySink::default();

    let entry = kernel
        .run_backup_to_sink(&sink, "0.3.0-test", BackupHistoryKind::Manual)
        .unwrap();
    assert!(entry.verified);
    assert_eq!(entry.error, None);
    assert_eq!(entry.kind, BackupHistoryKind::Manual);
    assert_eq!(entry.format_version, 2);
    assert!(entry.destination.starts_with("memory://test/"));

    // It is listed, with the size the receipt recorded.
    let listed = sink.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(entry.destination, sink.describe(&listed[0].name));
    assert_eq!(listed[0].size_bytes, entry.size_bytes);
    assert!(listed[0].size_bytes > 0);

    // The receipt is in the vault's own history.
    let history = kernel.backup_history().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].backup_id, entry.backup_id);

    // Restore-as-new from the bytes the sink returns, with no backup file path.
    let package = sink.get(&listed[0].name).unwrap();
    let target = tempdir().unwrap();
    let restored =
        Kernel::restore_backup_bytes(&package, PASSWORD, &target.path().join("vault.db")).unwrap();
    assert_eq!(
        restored.vault_metadata().unwrap().vault_id,
        kernel.vault_metadata().unwrap().vault_id
    );
    // A wrong password is refused before anything is written.
    let other = tempdir().unwrap();
    assert!(
        Kernel::restore_backup_bytes(&package, b"wrong", &other.path().join("vault.db")).is_err()
    );
    assert!(!other.path().join("vault.db").exists());
}

#[test]
fn every_run_adds_a_backup_and_nothing_is_ever_removed() {
    let (_root, kernel) = kernel();
    let sink = MemorySink::default();
    let mut names = Vec::new();
    for kind in [
        BackupHistoryKind::Scheduled,
        BackupHistoryKind::Manual,
        BackupHistoryKind::Scheduled,
    ] {
        kernel
            .run_backup_to_sink(&sink, "0.3.0-test", kind)
            .unwrap();
        let listed: Vec<String> = sink.list().unwrap().into_iter().map(|e| e.name).collect();
        // Every earlier name is still there: the interface has no delete, and a new
        // run never replaces an older item.
        for earlier in &names {
            assert!(listed.contains(earlier));
        }
        names = listed;
    }
    assert_eq!(names.len(), 3);
    assert_eq!(kernel.backup_history().unwrap().len(), 3);
}

#[test]
fn a_destination_that_refuses_the_write_records_a_safe_failed_receipt() {
    let (_root, kernel) = kernel();
    let sink = MemorySink {
        fail_put: true,
        ..MemorySink::default()
    };
    let error = kernel
        .run_backup_to_sink(&sink, "0.3.0-test", BackupHistoryKind::Scheduled)
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("Could not write the backup file."));

    let history = kernel.backup_history().unwrap();
    assert_eq!(history.len(), 1);
    assert!(!history[0].verified);
    assert_eq!(history[0].size_bytes, 0);
    assert_eq!(
        history[0].error.as_deref(),
        Some("Could not write the backup file.")
    );
    assert!(sink.list().unwrap().is_empty());
}

#[test]
fn damaged_bytes_read_back_from_a_destination_fail_verification() {
    let (_root, kernel) = kernel();
    let sink = MemorySink {
        corrupt_get: true,
        ..MemorySink::default()
    };
    let error = kernel
        .run_backup_to_sink(&sink, "0.3.0-test", BackupHistoryKind::Manual)
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("Could not verify the backup file."));

    let history = kernel.backup_history().unwrap();
    assert_eq!(history.len(), 1);
    assert!(!history[0].verified);
    assert_eq!(
        history[0].error.as_deref(),
        Some("Could not verify the backup file.")
    );
    // The written item is left alone for diagnosis, never deleted.
    assert_eq!(sink.list().unwrap().len(), 1);
}

#[test]
fn the_folder_sink_gives_the_same_receipt_shape_as_the_configured_folder() {
    // The folder destination through the public seam: a path-bearing description,
    // a listed file, and bytes that restore.
    let (_root, kernel) = kernel();
    let folder = tempdir().unwrap();
    let sink = FolderSink::new(folder.path());
    let entry = kernel
        .run_backup_to_sink(&sink, "0.3.0-test", BackupHistoryKind::Scheduled)
        .unwrap();
    assert!(entry.verified);
    assert!(std::path::Path::new(&entry.destination).is_file());
    assert_eq!(sink.list().unwrap().len(), 1);
    let target = tempdir().unwrap();
    let name = &sink.list().unwrap()[0].name;
    Kernel::restore_backup_bytes(
        &sink.get(name).unwrap(),
        PASSWORD,
        &target.path().join("vault.db"),
    )
    .unwrap();
}

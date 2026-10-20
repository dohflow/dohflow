//! Backup destinations behind a small interface (personal-cfo-7ie.15).
//!
//! A backup is one finished, encrypted container (ADR 0024, 0024-A) written to a
//! destination. The kernel used to know only one kind of destination, an absolute
//! folder path. [`BackupSink`] is the seam: the kernel assembles the container and
//! verifies what a sink returns, and the sink only stores, lists and returns bytes.
//! The folder destination, [`FolderSink`], is the first implementation and behaves as
//! the kernel's file code always did. A crate that wants another destination (a
//! system document provider, a hosted account) implements the trait itself; nothing
//! about it lives here.
//!
//! The container format and the unattended key model are unchanged. A sink only ever
//! sees ciphertext, never a key, and a destination's name or credential never reaches
//! a log: [`BackupSinkError`] carries no text and the history receipt keeps only the
//! string [`BackupSink::describe`] returns, inside the encrypted vault.

use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// How [`BackupSink::put`] treats an existing item of the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutMode {
    /// Create the item and fail if it already exists, so a name collision can never
    /// replace an earlier recovery point. Scheduled and on-demand backups use this.
    CreateNew,
    /// Create the item or replace it. A manual export to a file the user chose uses
    /// this.
    Replace,
}

/// A destination failure. It deliberately carries no text: a path, an account name or
/// a provider message must not travel into a log or a user-visible error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupSinkError;

impl fmt::Display for BackupSinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("backup destination error")
    }
}

impl std::error::Error for BackupSinkError {}

/// One stored backup, as a sink lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSinkEntry {
    /// The name the container was stored under (a plain file name, no separators).
    pub name: String,
    /// The stored size in bytes.
    pub size_bytes: u64,
}

/// A place backups are written to and read back from.
///
/// Implementations store opaque ciphertext. They must never delete or rewrite an
/// item except through [`PutMode::Replace`] on the name they are given: DohFlow keeps
/// every backup until the user removes it (S2a-0), and this interface offers no
/// delete.
pub trait BackupSink {
    /// The label recorded in the local backup history for the item `name`. A folder
    /// sink returns the full path; another sink returns whatever identifies the item
    /// to its own user. It is stored only inside the encrypted vault and is never
    /// logged.
    fn describe(&self, name: &str) -> String;

    /// Store a finished container under `name` and return the stored size in bytes.
    ///
    /// # Errors
    /// [`BackupSinkError`] if the item cannot be written, including a
    /// [`PutMode::CreateNew`] collision.
    fn put(&self, name: &str, package: &[u8], mode: PutMode) -> Result<u64, BackupSinkError>;

    /// Return the stored container named `name`, for verification or restore.
    ///
    /// # Errors
    /// [`BackupSinkError`] if the item is missing or unreadable.
    fn get(&self, name: &str) -> Result<Vec<u8>, BackupSinkError>;

    /// List the backups in this destination, ordered by name.
    ///
    /// # Errors
    /// [`BackupSinkError`] if the destination cannot be read.
    fn list(&self) -> Result<Vec<BackupSinkEntry>, BackupSinkError>;
}

/// The local folder destination (the kernel's original and default destination).
#[derive(Debug, Clone)]
pub struct FolderSink {
    folder: PathBuf,
}

impl FolderSink {
    /// A sink over `folder`. The caller has validated that it is an absolute,
    /// existing directory (`configure_backup_schedule` does for the saved folder).
    #[must_use]
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self {
            folder: folder.into(),
        }
    }

    /// The item's path, or `None` unless `name` is a single plain file name: a name
    /// with a separator, `.` or `..` must never reach outside the folder.
    fn item_path(&self, name: &str) -> Option<PathBuf> {
        let mut components = Path::new(name).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => Some(self.folder.join(name)),
            _ => None,
        }
    }
}

impl BackupSink for FolderSink {
    fn describe(&self, name: &str) -> String {
        self.folder.join(name).to_string_lossy().into_owned()
    }

    fn put(&self, name: &str, package: &[u8], mode: PutMode) -> Result<u64, BackupSinkError> {
        let path = self.item_path(name).ok_or(BackupSinkError)?;
        let mut options = OpenOptions::new();
        options.write(true);
        match mode {
            PutMode::CreateNew => options.create_new(true),
            PutMode::Replace => options.create(true).truncate(true),
        };
        let mut file = options.open(path).map_err(|_| BackupSinkError)?;
        file.write_all(package).map_err(|_| BackupSinkError)?;
        Ok(package.len() as u64)
    }

    fn get(&self, name: &str) -> Result<Vec<u8>, BackupSinkError> {
        let path = self.item_path(name).ok_or(BackupSinkError)?;
        std::fs::read(path).map_err(|_| BackupSinkError)
    }

    fn list(&self) -> Result<Vec<BackupSinkEntry>, BackupSinkError> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&self.folder).map_err(|_| BackupSinkError)? {
            let entry = entry.map_err(|_| BackupSinkError)?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("pcfobk") {
                continue;
            }
            let metadata = entry.metadata().map_err(|_| BackupSinkError)?;
            let (true, Some(name)) = (
                metadata.is_file(),
                entry.file_name().to_str().map(str::to_owned),
            ) else {
                continue;
            };
            entries.push(BackupSinkEntry {
                name,
                size_bytes: metadata.len(),
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn a_name_that_is_not_a_plain_file_name_is_refused() {
        let dir = tempdir().unwrap();
        let sink = FolderSink::new(dir.path());
        for name in [
            "",
            ".",
            "..",
            "../escape.pcfobk",
            "a/b.pcfobk",
            "/abs.pcfobk",
        ] {
            assert_eq!(
                sink.put(name, b"x", PutMode::CreateNew),
                Err(BackupSinkError)
            );
            assert_eq!(sink.get(name), Err(BackupSinkError));
        }
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn create_new_never_replaces_and_replace_does() {
        let dir = tempdir().unwrap();
        let sink = FolderSink::new(dir.path());
        assert_eq!(sink.put("one.pcfobk", b"first", PutMode::CreateNew), Ok(5));
        assert_eq!(
            sink.put("one.pcfobk", b"second", PutMode::CreateNew),
            Err(BackupSinkError)
        );
        assert_eq!(sink.get("one.pcfobk").unwrap(), b"first");
        assert_eq!(sink.put("one.pcfobk", b"third!", PutMode::Replace), Ok(6));
        assert_eq!(sink.get("one.pcfobk").unwrap(), b"third!");
    }

    #[test]
    fn list_returns_only_backup_files_in_name_order() {
        let dir = tempdir().unwrap();
        let sink = FolderSink::new(dir.path());
        sink.put("b.pcfobk", b"bb", PutMode::CreateNew).unwrap();
        sink.put("a.pcfobk", b"a", PutMode::CreateNew).unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"not a backup").unwrap();
        std::fs::create_dir(dir.path().join("sub.pcfobk")).unwrap();
        assert_eq!(
            sink.list().unwrap(),
            vec![
                BackupSinkEntry {
                    name: "a.pcfobk".to_owned(),
                    size_bytes: 1
                },
                BackupSinkEntry {
                    name: "b.pcfobk".to_owned(),
                    size_bytes: 2
                },
            ]
        );
    }

    #[test]
    fn the_error_carries_no_text() {
        assert_eq!(BackupSinkError.to_string(), "backup destination error");
    }
}

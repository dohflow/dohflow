//! The write-ahead-log header guard (personal-cfo-5vto5, RUSTSEC-2026-0459).
//!
//! SQLite before 3.54.0 can return uninitialized page-cache memory when it opens a database together
//! with a sibling `-wal` file whose page size differs from the database's. The trigger is local: someone
//! has to place the file beside the database. SQLCipher has no build on a fixed SQLite yet, so every
//! SQLCipher open of a vault, a base image, a rebuilt copy or a restored backup first reads the first
//! twelve bytes of a sibling `-wal` and refuses when its page size is not the one every vault here uses.
//!
//! The guard only READS. A refused file is left exactly where it is: not moved, renamed, truncated or
//! deleted, because a genuine WAL can hold committed transactions that have not been checkpointed yet.
//! The person is told what was found and decides.
//!
//! Every connection to a vault file goes through [`crate::open_vault_connection`], which calls
//! [`check_wal_header`] before it opens anything; a source scan in the tests fails if another call to
//! SQLite's open functions appears.

use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::DbError;

/// The page size of every vault, base image and backup this app opens: SQLCipher 4's default, which no
/// build here overrides. A test opens a real vault and checks the library still reports it.
pub(crate) const VAULT_PAGE_SIZE: u32 = 4096;

/// The `-wal` file SQLite would pair with the database at `path`.
fn wal_path(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push("-wal");
    PathBuf::from(name)
}

/// Refuses to continue when a sibling `-wal` of `path` has a header whose page size (bytes 8 to 11,
/// big-endian) is not [`VAULT_PAGE_SIZE`].
///
/// No `-wal` is fine. So is one shorter than twelve bytes: SQLite cannot read a header from it and
/// ignores it, and it holds no page size to compare. A `-wal` that exists but cannot be read is refused
/// (an I/O error), because it cannot be shown to be safe. Nothing is ever written.
///
/// # Errors
/// [`DbError::WalHeaderMismatch`] with the page size found, or [`DbError::Io`] for a `-wal` that cannot
/// be read.
pub(crate) fn check_wal_header(path: &Path) -> Result<(), DbError> {
    let wal = wal_path(path);
    let file = match File::open(&wal) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(DbError::Io(error)),
    };
    let mut header = Vec::with_capacity(12);
    file.take(12).read_to_end(&mut header)?;
    let Some(page_size) = header.get(8..12) else {
        return Ok(());
    };
    let found = u32::from_be_bytes([page_size[0], page_size[1], page_size[2], page_size[3]]);
    if found == VAULT_PAGE_SIZE {
        Ok(())
    } else {
        Err(DbError::WalHeaderMismatch {
            found_page_size: found,
            expected_page_size: VAULT_PAGE_SIZE,
        })
    }
}

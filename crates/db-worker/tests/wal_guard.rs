//! The write-ahead-log header guard (personal-cfo-5vto5, RUSTSEC-2026-0459): a sibling `-wal` whose page
//! size is not the one every vault uses is refused before SQLite sees it, and the file is left exactly
//! where it is. Synthetic vaults only.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{create_account_cmd, meta, KEY};
use db_worker::{DbError, DbWorker};
use tempfile::TempDir;

fn wal_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push("-wal");
    PathBuf::from(name)
}

/// A vault that still has a real `-wal` with committed work in it, copied (main file and `-wal`) into a
/// fresh directory, the way a crash or a copied vault folder leaves them. Returns the directory and the
/// copy's path.
fn vault_with_a_live_wal() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("source.vault");
    let worker = DbWorker::open(&source, KEY).unwrap();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let wal = wal_of(&source);
    assert!(
        fs::metadata(&wal).map(|m| m.len() > 32).unwrap_or(false),
        "the live vault has a -wal with frames in it"
    );
    let copy_dir = TempDir::new().unwrap();
    let copy = copy_dir.path().join("copy.vault");
    fs::copy(&source, &copy).unwrap();
    fs::copy(&wal, wal_of(&copy)).unwrap();
    drop(worker);
    (copy_dir, copy)
}

/// Every file in `dir` with its bytes, so "left exactly in place" is a comparison, not a belief.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = fs::read_dir(dir)
        .unwrap()
        // The runner's own lock file is created by the open attempt before the guard runs; it holds no
        // data and is not the vault or its log.
        .filter(|entry| {
            !entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".runner.lock")
        })
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap_or_default(),
            )
        })
        .collect();
    files.sort();
    files
}

/// `snapshot` equality with a readable failure: the file names and sizes, and the first byte that differs.
fn assert_unchanged(dir: &Path, before: &[(String, Vec<u8>)]) {
    let after = snapshot(dir);
    let names = |files: &[(String, Vec<u8>)]| -> Vec<(String, usize)> {
        files.iter().map(|(n, b)| (n.clone(), b.len())).collect()
    };
    assert_eq!(names(&after), names(before), "the files and their sizes");
    for ((name, now), (_, then)) in after.iter().zip(before) {
        let first = now.iter().zip(then).position(|(a, b)| a != b);
        assert_eq!(first, None, "{name} changed (first differing byte)");
    }
}

fn patch_wal_page_size(wal: &Path, page_size: u32) {
    let mut bytes = fs::read(wal).unwrap();
    bytes[8..12].copy_from_slice(&page_size.to_be_bytes());
    fs::write(wal, bytes).unwrap();
}

#[test]
fn a_real_wal_names_the_page_size_every_vault_uses() {
    // The constant the guard compares against is checked against what SQLCipher really writes.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let worker = DbWorker::open(&path, KEY).unwrap();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let header = fs::read(wal_of(&path)).unwrap();
    assert!(
        matches!(
            header[0..4],
            [0x37, 0x7f, 0x06, 0x82] | [0x37, 0x7f, 0x06, 0x83]
        ),
        "a WAL magic number"
    );
    assert_eq!(&header[8..12], &4096_u32.to_be_bytes(), "the WAL page size");
    let conn = worker.read_connection().unwrap();
    // The pragmas answer as numbers or as text, depending on the library's mood.
    let pragma = |name: &str| -> i64 {
        conn.query_row(&format!("PRAGMA {name}"), [], |row| {
            Ok(match row.get_ref(0)? {
                rusqlite::types::ValueRef::Integer(n) => n,
                rusqlite::types::ValueRef::Text(t) => {
                    String::from_utf8_lossy(t).parse().unwrap_or(-1)
                }
                _ => -1,
            })
        })
        .unwrap()
    };
    let (cipher, page) = (pragma("cipher_page_size"), pragma("page_size"));
    assert_eq!((cipher, page), (4096, 4096), "the library's page size");
}

#[test]
fn a_matching_wal_opens_and_its_committed_work_is_there() {
    let (_dir, copy) = vault_with_a_live_wal();
    let worker = DbWorker::open(&copy, KEY).unwrap();
    let accounts: i64 = worker
        .read_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
        .unwrap();
    assert!(accounts >= 1, "the account committed in the WAL is read");
}

#[test]
fn a_mismatched_wal_is_refused_and_everything_is_left_byte_identical() {
    let (dir, copy) = vault_with_a_live_wal();
    patch_wal_page_size(&wal_of(&copy), 8192);
    let before = snapshot(dir.path());

    // Every way into a vault goes through the guard.
    let by_passphrase = DbWorker::open(&copy, KEY).err().expect("refused");
    assert!(
        matches!(
            by_passphrase,
            DbError::WalHeaderMismatch {
                found_page_size: 8192,
                expected_page_size: 4096
            }
        ),
        "{by_passphrase:?}"
    );
    assert_eq!(
        snapshot(dir.path()),
        before,
        "nothing was written or created"
    );

    let dek = vault_crypto::generate_dek().unwrap();
    let by_key = DbWorker::open_existing_with_raw_key(&copy, dek)
        .err()
        .expect("refused");
    assert!(
        matches!(by_key, DbError::WalHeaderMismatch { .. }),
        "{by_key:?}"
    );
    assert_unchanged(dir.path(), &before);

    // The sizes are the whole message: no path.
    let text = by_passphrase.to_string();
    assert!(text.contains("8192") && text.contains("4096"), "{text}");
    assert!(!text.contains("copy.vault"), "no path in {text}");
}

#[test]
fn a_wal_one_byte_off_in_the_size_is_refused_too() {
    let (dir, copy) = vault_with_a_live_wal();
    patch_wal_page_size(&wal_of(&copy), 4097);
    let before = snapshot(dir.path());
    assert!(matches!(
        DbWorker::open(&copy, KEY).err().expect("refused"),
        DbError::WalHeaderMismatch {
            found_page_size: 4097,
            ..
        }
    ));
    assert_unchanged(dir.path(), &before);
}

#[test]
fn no_wal_opens() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    drop(DbWorker::open(&path, KEY).unwrap());
    assert!(!wal_of(&path).exists(), "a closed vault has no -wal");
    DbWorker::open(&path, KEY).unwrap();
}

#[test]
fn an_empty_wal_is_not_a_log_and_opens() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    drop(DbWorker::open(&path, KEY).unwrap());
    fs::write(wal_of(&path), []).unwrap();
    DbWorker::open(&path, KEY).unwrap();
}

#[test]
fn a_wal_that_cannot_be_read_is_refused_not_trusted() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    drop(DbWorker::open(&path, KEY).unwrap());
    // A directory where the log should be: it cannot be shown to be safe.
    fs::create_dir(wal_of(&path)).unwrap();
    let error = DbWorker::open(&path, KEY).err().expect("refused");
    assert!(matches!(error, DbError::Io(_)), "{error:?}");
    assert!(wal_of(&path).is_dir(), "left in place");
}

#[cfg(unix)]
#[test]
fn a_wal_the_app_may_not_read_is_refused_not_ignored() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, copy) = vault_with_a_live_wal();
    let wal = wal_of(&copy);
    fs::set_permissions(&wal, fs::Permissions::from_mode(0o000)).unwrap();
    // Running as root would read it anyway; then there is nothing to refuse.
    if fs::File::open(&wal).is_ok() {
        fs::set_permissions(&wal, fs::Permissions::from_mode(0o600)).unwrap();
        return;
    }
    let error = DbWorker::open(&copy, KEY).err().expect("refused");
    fs::set_permissions(&wal, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(error, DbError::Io(_)), "{error:?}");
    let _ = dir;
}

// ---- no open bypasses the guard ------------------------------------------------------------------------

/// Every `.rs` file under `dir`, skipping test directories and the test-only tail of a file.
fn production_sources(dir: &Path, found: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "tests" && name != "target" && name != "node_modules" && name != "benches" {
                production_sources(&path, found);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") && name != "tests.rs" {
            let text = fs::read_to_string(&path).unwrap();
            // Cut at an inline test module (`#[cfg(test)]` before `mod name {`), at column 0.
            let lines: Vec<&str> = text.lines().collect();
            let cut = lines
                .windows(2)
                .position(|pair| {
                    pair[0] == "#[cfg(test)]"
                        && pair[1].starts_with("mod ")
                        && pair[1].ends_with('{')
                })
                .unwrap_or(lines.len());
            found.push((path, lines[..cut].join("\n")));
        }
    }
}

#[test]
fn every_connection_to_a_vault_file_goes_through_the_guard() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sources = Vec::new();
    for dir in ["crates", "apps/desktop/src-tauri/src"] {
        let base = root.join(dir);
        if dir == "crates" {
            for crate_dir in fs::read_dir(&base).unwrap() {
                let src = crate_dir.unwrap().path().join("src");
                if src.is_dir() {
                    production_sources(&src, &mut sources);
                }
            }
        } else {
            production_sources(&base, &mut sources);
        }
    }
    assert!(sources.len() > 50, "the scan found the sources");

    // SQLite's open functions as rusqlite spells them. `open_in_memory` has no file, so no log.
    let openers = [
        "Connection::open(",
        "Connection::open_with_flags(",
        "Connection::open_with_flags_and_vfs(",
        "Connection::open_with_vfs(",
        "open_with_flags_and_vfs(",
    ];
    let mut hits = Vec::new();
    for (path, text) in &sources {
        for (number, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if openers.iter().any(|opener| code.contains(opener)) {
                hits.push((
                    path.strip_prefix(&root)
                        .unwrap_or(path)
                        .display()
                        .to_string(),
                    number + 1,
                    line.trim().to_owned(),
                ));
            }
        }
    }
    assert_eq!(
        hits.len(),
        1,
        "exactly one place opens a SQLite file, and it is the guarded helper: {hits:#?}"
    );
    assert_eq!(hits[0].0.replace('\\', "/"), "crates/db-worker/src/lib.rs");

    // And that helper checks the header before it opens anything.
    let lib = fs::read_to_string(root.join("crates/db-worker/src/lib.rs")).unwrap();
    let helper = lib
        .split("fn open_vault_connection(")
        .nth(1)
        .expect("the helper")
        .split("\n}\n")
        .next()
        .unwrap();
    let guard = helper
        .find("wal_guard::check_wal_header(")
        .expect("the guard call");
    let open = helper
        .find("Connection::open_with_flags(")
        .expect("the open");
    assert!(guard < open, "the guard runs before the open");
}

//! Proves ADR 0070 / `personal-cfo-he3xo`'s central safety property at the filesystem
//! level, not just at the pure-function level already covered by `data_dir`'s own unit
//! tests: setting up a dev-channel run must never touch a pre-existing "release" vault
//! directory, and must land in a distinct sibling.
//!
//! This is the automated equivalent of the bead's manual acceptance line ("running
//! `pnpm dev` while `/Applications/DohFlow.app` holds a vault leaves that vault
//! untouched, verified by hash before and after") — using real file IO and a real
//! content hash against synthetic paths, so it runs in CI on every PR instead of once
//! by hand.

use std::fs;
use std::path::Path;

use app_lib::data_dir::resolve_data_dir;

/// Cheap, sufficient content fingerprint for this test — not a security primitive,
/// just a way to prove a file's bytes did not change.
fn fingerprint(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"))
}

#[test]
fn dev_channel_setup_never_touches_the_release_directory() {
    let support_root = tempfile::tempdir().expect("temp dir");
    let release_dir = support_root.path().join("ai.personalcfo.desktop");
    fs::create_dir_all(&release_dir).unwrap();

    // Stand in for a real, already-unlocked vault the release app holds.
    let release_vault = release_dir.join("vault.db");
    fs::write(&release_vault, b"pretend-encrypted-vault-bytes-v1").unwrap();
    let release_files_before: Vec<_> = fs::read_dir(&release_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let before = fingerprint(&release_vault);

    // Exactly what `lib.rs`'s `setup()` does for a dev-channel run.
    let dev_dir = resolve_data_dir("dev", true, release_dir.clone(), None);
    fs::create_dir_all(&dev_dir).unwrap();
    // And exactly what opening/creating a fresh vault there would do.
    fs::write(
        dev_dir.join("vault.db"),
        b"a-completely-different-dev-vault",
    )
    .unwrap();

    let after = fingerprint(&release_vault);
    assert_eq!(before, after, "the release vault's bytes must be untouched");

    let release_files_after: Vec<_> = fs::read_dir(&release_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        release_files_before, release_files_after,
        "no new files (WAL/SHM/etc.) may appear in the release directory"
    );

    assert_ne!(dev_dir, release_dir);
    assert_eq!(
        dev_dir.file_name().unwrap().to_str().unwrap(),
        "ai.personalcfo.desktop-dev"
    );
}

#[test]
fn release_channel_setup_ignores_a_stray_override_env_shape() {
    // Simulates someone accidentally carrying a PCFO_DATA_DIR from a dev shell
    // into a release-channel run: the override argument must be inert.
    let support_root = tempfile::tempdir().expect("temp dir");
    let release_dir = support_root.path().join("ai.personalcfo.desktop");
    let stray_override = support_root.path().join("leftover-dev-override");
    fs::create_dir_all(&release_dir).unwrap();

    let resolved = resolve_data_dir("release", false, release_dir.clone(), Some(stray_override));
    assert_eq!(resolved, release_dir);
}

#[test]
fn a_debug_build_mislabeled_as_a_non_dev_channel_still_never_touches_the_release_directory() {
    // personal-cfo-qrh3t, ADR 0070 addendum: the gap this closes. Before it,
    // `PCFO_BUILD_CHANNEL=beta pnpm tauri dev` — a debug-profile binary whose
    // channel label is not "dev" — was treated as release and pointed
    // straight at the real vault directory. Same filesystem-level proof as
    // `dev_channel_setup_never_touches_the_release_directory` above, but
    // with the channel label that used to leak through.
    let support_root = tempfile::tempdir().expect("temp dir");
    let release_dir = support_root.path().join("ai.personalcfo.desktop");
    fs::create_dir_all(&release_dir).unwrap();

    let release_vault = release_dir.join("vault.db");
    fs::write(&release_vault, b"pretend-encrypted-vault-bytes-v1").unwrap();
    let before = fingerprint(&release_vault);

    let dev_dir = resolve_data_dir("beta", true, release_dir.clone(), None);
    fs::create_dir_all(&dev_dir).unwrap();
    fs::write(
        dev_dir.join("vault.db"),
        b"a-completely-different-dev-vault",
    )
    .unwrap();

    let after = fingerprint(&release_vault);
    assert_eq!(before, after, "the release vault's bytes must be untouched");
    assert_ne!(dev_dir, release_dir);
}

//! End-to-end encrypted backup export (personal-cfo-ef3, ADR 0024). Creates a
//! real vault, exports it, and asserts the package is **sealed** (the on-disk
//! `vault.db` ciphertext does not appear in the clear), **round-trips** under the
//! right password (recovering the exact files, hashes verified), and **refuses**
//! the wrong one.

use finance_kernel::backup::disassemble;
use finance_kernel::Kernel;
use uuid::Uuid;

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn export_produces_a_sealed_package_that_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("vault.db");
    let kernel = Kernel::create_vault(&db_path, b"correct horse battery").unwrap();

    let out = dir.path().join("backup.pcfobk");
    kernel
        .export_unattended(
            &out,
            "0.1.0-test",
            "2026-06-20T00:00:00Z".into(),
            Uuid::from_bytes([9u8; 16]),
        )
        .unwrap();

    let package = std::fs::read(&out).unwrap();
    let db_bytes = std::fs::read(&db_path).unwrap();
    let envelope_bytes = std::fs::read(format!("{}.envelope", db_path.display())).unwrap();
    assert!(db_bytes.len() > 200, "expected a non-trivial vault.db");

    // The payload is sealed (AES-256-GCM): a slice of the real vault.db ciphertext
    // must NOT appear verbatim in the package.
    let mid = db_bytes.len() / 2;
    assert!(
        !contains(&package, &db_bytes[mid..mid + 64]),
        "vault.db bytes appear unsealed in the package"
    );

    // Round-trips under the right password, recovering the exact files.
    let restored = disassemble(b"correct horse battery", &package).unwrap();
    assert_eq!(restored.db_bytes, db_bytes);
    assert_eq!(restored.envelope_bytes, envelope_bytes);
    assert_eq!(
        restored.manifest.schema_version,
        finance_kernel::CURRENT_SCHEMA_VERSION
    );
    assert_eq!(restored.manifest.manifest_schema_version, Some(1));
    assert_eq!(
        restored.manifest.backup_id,
        Uuid::from_bytes([9u8; 16]).to_string()
    );

    // The wrong password cannot open it.
    assert!(disassemble(b"wrong password", &package).is_err());
}

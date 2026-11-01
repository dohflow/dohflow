//! The value types: validation, redaction and the matrix document in step with the code.

use platform_traits::matrix::{Platform, Support, MATRIX};
use platform_traits::{
    AccessGroup, AccessPolicy, AuthPrompt, SecretError, SecretKey, SecretValue, MAX_PROMPT_CHARS,
    MAX_SECRET_BYTES,
};

const CANARY: &[u8] = b"CANARY-secret-bytes-7f3a";

#[test]
fn a_secret_value_never_prints_its_bytes() {
    let value = SecretValue::new(CANARY).unwrap();
    let text = format!("{value:?} {value:#?}");
    assert!(!text.contains("CANARY"), "{text}");
    assert!(!text.contains(&format!("{:?}", CANARY)), "{text}");
    assert!(text.contains("<redacted, 24 bytes>"), "{text}");
}

#[test]
fn errors_and_keys_never_carry_a_value() {
    // The error type has no field that could hold bytes; show every Display once.
    for error in [
        SecretError::NotFound,
        SecretError::InvalidKey,
        SecretError::InvalidPrompt,
        SecretError::TooLarge,
        SecretError::Empty,
        SecretError::AuthCancelled,
        SecretError::AuthFailed,
        SecretError::BiometryNotEnrolled,
        SecretError::BiometryLockedOut,
        SecretError::Unavailable(platform_traits::UnavailableReason::MissingEntitlement),
        SecretError::Platform { code: -34018 },
    ] {
        let text = format!("{error} / {error:?}");
        assert!(!text.is_empty());
        assert!(!text.contains("CANARY"));
    }
}

#[test]
fn secret_value_bounds_and_equality() {
    assert_eq!(SecretValue::new(b"").unwrap_err(), SecretError::Empty);
    assert_eq!(
        SecretValue::from_vec(Vec::new()).unwrap_err(),
        SecretError::Empty
    );
    assert!(SecretValue::new(&vec![1_u8; MAX_SECRET_BYTES]).is_ok());
    assert_eq!(
        SecretValue::new(&vec![1_u8; MAX_SECRET_BYTES + 1]).unwrap_err(),
        SecretError::TooLarge
    );
    let a = SecretValue::new(b"same").unwrap();
    assert_eq!(a, a.duplicate());
    assert_ne!(a, SecretValue::new(b"samf").unwrap());
    assert_ne!(a, SecretValue::new(b"same!").unwrap());
    assert_eq!(a.len(), 4);
}

#[test]
fn secret_keys_are_validated_and_printable() {
    let key = SecretKey::new("app.dohflow.vault", "0190-abcd_1").unwrap();
    assert_eq!(format!("{key}"), "app.dohflow.vault/0190-abcd_1");
    assert_eq!(
        format!("{key:?}"),
        "SecretKey(app.dohflow.vault/0190-abcd_1)"
    );
    for (service, account) in [
        ("vault", "x"),            // missing the app.dohflow. prefix
        ("app.dohflow.vault", ""), // empty account
        ("app.dohflow.vault", "has space"),
        ("app.dohflow.vault", "slash/er"),
        ("app.dohflow.vault", "nul\0"),
        ("app.dohflow.vault", "ünïcode"),
        ("app.dohflow.", ""),
    ] {
        assert_eq!(
            SecretKey::new(service, account).unwrap_err(),
            SecretError::InvalidKey,
            "{service}/{account}"
        );
    }
    let long = "a".repeat(129);
    assert!(SecretKey::new("app.dohflow.vault", &long).is_err());
    assert!(SecretKey::new("app.dohflow.vault", &"a".repeat(128)).is_ok());
}

#[test]
fn access_groups_must_be_team_prefixed() {
    assert!(AccessGroup::new("SKU2HZ2U6L.app.dohflow.desktop").is_ok());
    for bad in [
        "app.dohflow.desktop",
        "sku2hz2u6l.app.dohflow.desktop",
        "SKU2HZ2U6.x",
        "SKU2HZ2U6L",
        "SKU2HZ2U6L.",
        "SKU2HZ2U6L.a b",
        "",
    ] {
        assert!(AccessGroup::new(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn prompts_are_bounded_text() {
    assert!(AuthPrompt::new("Unlock your vault").is_ok());
    assert!(AuthPrompt::new(&"x".repeat(MAX_PROMPT_CHARS)).is_ok());
    for bad in [
        "",
        "   ",
        "line\nbreak",
        "tab\t",
        &"x".repeat(MAX_PROMPT_CHARS + 1),
    ] {
        // A prompt problem is its own typed kind, not a key problem (xuu N1).
        assert_eq!(
            AuthPrompt::new(bad).unwrap_err(),
            SecretError::InvalidPrompt,
            "{bad:?}"
        );
    }
    assert_eq!(AccessPolicy::DeviceOnly, AccessPolicy::DeviceOnly);
}

#[test]
fn the_matrix_document_matches_the_code() {
    let doc = include_str!("../../../docs/architecture/platform-capability-matrix.md");
    for row in MATRIX {
        assert!(
            doc.contains(row.capability),
            "capability missing from the document: {}",
            row.capability
        );
        for platform in [
            Platform::MacOs,
            Platform::Ios,
            Platform::Linux,
            Platform::Windows,
            Platform::Android,
        ] {
            let cell = row.on(platform);
            assert!(
                !cell.note.is_empty(),
                "{} has an empty note",
                row.capability
            );
            assert!(
                doc.contains(cell.note),
                "note missing from the document: {}",
                cell.note
            );
        }
    }
    // The anti-rollback row says No everywhere: a keychain is never the witness.
    let witness = MATRIX
        .iter()
        .find(|r| r.capability.starts_with("Anti-rollback"))
        .unwrap();
    for platform in [
        Platform::MacOs,
        Platform::Ios,
        Platform::Linux,
        Platform::Windows,
        Platform::Android,
    ] {
        assert_eq!(witness.on(platform).support, Support::No);
    }
}

//! Source-scan guards for Sync key custody (personal-cfo-1cltt.2). Each one pins a rule that a
//! reviewer would otherwise have to re-read the code to check.

const KEYS: &str = include_str!("../src/sync_keys.rs");
const KEYSET: &str = include_str!("../src/sync_keyset.rs");
const RECORDS: &str = include_str!("../src/sync_records.rs");
const CUSTODY: &str = include_str!("../src/sync_custody.rs");
const SYNC: &str = include_str!("../src/sync.rs");
const ENVELOPE: &str = include_str!("../src/sync_envelope.rs");

const CUSTODY_FILES: [(&str, &str); 4] = [
    ("sync_keys.rs", KEYS),
    ("sync_keyset.rs", KEYSET),
    ("sync_records.rs", RECORDS),
    ("sync_custody.rs", CUSTODY),
];

/// Production code only: the text before the file's `#[cfg(test)]` module, with comments removed.
fn code(source: &str) -> String {
    let end = source.find("#[cfg(test)]").unwrap_or(source.len());
    source[..end]
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn nothing_in_key_custody_logs_or_prints() {
    for (name, source) in CUSTODY_FILES {
        let code = code(source);
        for forbidden in [
            "tracing",
            "println!",
            "eprintln!",
            "dbg!",
            "log::",
            "print!(",
        ] {
            assert!(!code.contains(forbidden), "{name} uses {forbidden}");
        }
    }
}

#[test]
fn randomness_comes_from_the_operating_system_only() {
    for (name, source) in CUSTODY_FILES {
        let code = code(source);
        for forbidden in [
            "thread_rng",
            "OsRng",
            "rand::",
            "StdRng",
            "SmallRng",
            "from_entropy",
        ] {
            assert!(!code.contains(forbidden), "{name} uses {forbidden}");
        }
    }
    // The single generator type handed to hpke yields one fixed seed and is not a general RNG.
    assert!(code(KEYSET).contains("impl CryptoRng for SeedRng"));
    assert_eq!(code(KEYSET).matches("CryptoRng for").count(), 1);
}

#[test]
fn the_seed_taking_seal_and_the_raw_open_are_used_nowhere_but_the_keyset_module() {
    for (name, source) in [
        ("sync_keys.rs", KEYS),
        ("sync_records.rs", RECORDS),
        ("sync_custody.rs", CUSTODY),
        ("sync.rs", SYNC),
        ("sync_envelope.rs", ENVELOPE),
    ] {
        let code = code(source);
        assert!(
            !code.contains("seal_with_seed"),
            "{name} calls seal_with_seed"
        );
        assert!(
            !code.contains("open_with_key"),
            "{name} calls open_with_key"
        );
    }
}

#[test]
fn a_struct_that_holds_key_bytes_does_not_derive_debug() {
    for (name, source) in [
        ("sync_keys.rs", KEYS),
        ("sync_keyset.rs", KEYSET),
        ("sync_records.rs", RECORDS),
        ("sync_custody.rs", CUSTODY),
        ("sync.rs", SYNC),
        ("sync_envelope.rs", ENVELOPE),
    ] {
        let code = code(source);
        let lines: Vec<&str> = code.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let is_struct = line.trim_start().starts_with("pub struct ")
                || line.trim_start().starts_with("struct ");
            if !is_struct || !line.trim_end().ends_with('{') {
                continue;
            }
            let body: Vec<&&str> = lines[i + 1..]
                .iter()
                .take_while(|l| !l.starts_with('}') && !l.trim_start().starts_with("}"))
                .collect();
            let holds_secret = body.iter().any(|l| l.contains("SecretBytes"));
            if !holds_secret {
                continue;
            }
            let attributes: String = lines[..i]
                .iter()
                .rev()
                .take_while(|l| {
                    l.trim_start().starts_with("#[") || l.trim_start().starts_with("///")
                })
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                !attributes.contains("Debug"),
                "{name}: `{}` holds key bytes and derives Debug",
                line.trim()
            );
        }
    }
}

#[test]
fn the_x25519_dalek_pin_and_its_zeroize_feature_are_in_the_manifest() {
    let manifest = include_str!("../../../Cargo.toml");
    assert!(manifest.contains("x25519-dalek = { version = \"=2.0.1\""));
    assert!(manifest.contains("\"zeroize\""));
    assert!(manifest.contains("hpke = { version = \"=0.13.0\", default-features = false"));
    assert!(manifest.contains("ed25519-dalek = { version = \"=2.2.0\""));
}

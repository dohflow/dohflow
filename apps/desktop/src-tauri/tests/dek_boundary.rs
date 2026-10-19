//! The DEK never crosses IPC (personal-cfo-7ie.10, acceptance criterion 1; ADR 0002
//! decision 6; condition H2).
//!
//! `Kernel::unlock_vault_with_dek` takes a DEK released by the platform's biometric store.
//! That key is released by Rust platform code straight into the kernel: no Tauri command may
//! accept it as an argument or return it, and no IPC type may carry it. This is a source-level
//! contract over the registered command list and the whole IPC module: it fails if a command
//! signature, an IPC DTO or the IPC code names a key-bearing type or the DEK entry point.

use std::collections::BTreeSet;

// Only the command parser is used here; the rest of the shared audit module is for other tests.
#[allow(dead_code)]
#[path = "support/capability_audit.rs"]
mod capability_audit;

const LIB_RS: &str = include_str!("../src/lib.rs");
const IPC_COMMANDS: &str = include_str!("../src/ipc/commands.rs");
const IPC_DTO: &str = include_str!("../src/ipc/dto.rs");
const IPC_ERROR: &str = include_str!("../src/ipc/error.rs");
const IPC_MOD: &str = include_str!("../src/ipc/mod.rs");

/// Tokens that mean key material or the entry point that consumes it.
const FORBIDDEN: &[&str] = &[
    "SecretBytes",
    "ReleasedUnlockMaterial",
    "RELEASED_LEN",
    "unlock_vault_with_dek",
    "released_unlock",
    "dek_fingerprint",
    "vault_crypto::Dek",
    "expose_bytes",
];

/// Whole-word, case-insensitive: `dek` as an identifier or type (`Dek`, `dek`, `DEK`).
fn mentions_dek_word(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(found) = lower[from..].find("dek") {
        let at = from + found;
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(at + 3).copied();
        let boundary = |b: Option<u8>| b.is_none_or(|c| !c.is_ascii_alphanumeric());
        if boundary(before) && boundary(after) {
            return Some(at);
        }
        from = at + 3;
    }
    None
}

#[test]
fn the_command_list_is_the_real_one() {
    let commands = capability_audit::registered_commands(LIB_RS);
    assert!(commands.len() > 100, "parsed {} commands", commands.len());
}

#[test]
fn no_ipc_module_names_key_material_or_the_dek_entry_point() {
    for (file, text) in [
        ("ipc/commands.rs", IPC_COMMANDS),
        ("ipc/dto.rs", IPC_DTO),
        ("ipc/error.rs", IPC_ERROR),
        ("ipc/mod.rs", IPC_MOD),
    ] {
        for token in FORBIDDEN {
            assert!(
                !text.contains(token),
                "{file} names `{token}`: the DEK must not cross IPC"
            );
        }
        // A `dek` identifier is allowed nowhere in the IPC layer, in code or in DTOs.
        for (number, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            assert!(
                mentions_dek_word(code).is_none(),
                "{file}:{}: `{}` mentions a DEK",
                number + 1,
                line.trim()
            );
        }
    }
}

#[test]
fn no_registered_command_takes_or_returns_key_bytes() {
    let registered: BTreeSet<String> = capability_audit::registered_commands(LIB_RS);
    let mut checked = 0;
    let mut raw_byte_commands = BTreeSet::new();
    for name in &registered {
        let needle = format!("fn {name}(");
        let Some(start) = IPC_COMMANDS.find(&needle) else {
            continue;
        };
        // The signature runs from `fn name(` to the first `{` that opens the body.
        let signature = &IPC_COMMANDS[start..start + IPC_COMMANDS[start..].find('{').unwrap()];
        for token in FORBIDDEN {
            assert!(
                !signature.contains(token),
                "{name} signature names `{token}`"
            );
        }
        assert!(
            mentions_dek_word(signature).is_none(),
            "{name} signature mentions a DEK: {signature}"
        );
        // A raw byte buffer as a command argument or return is where key bytes would have to
        // travel, so every such command is named here and reviewed: a new one fails this test.
        if signature.contains("Vec<u8>") || signature.contains("[u8;") {
            raw_byte_commands.insert(name.clone());
        }
        checked += 1;
    }
    assert!(
        checked > 100,
        "only {checked} command signatures were inspected"
    );
    // Reviewed: `attach_document` carries a user's document bytes into the encrypted
    // attachment store, and the two `import_preview_*` commands carry the bytes of a file the
    // user picked for import. None is key material.
    let reviewed: BTreeSet<String> = [
        "attach_document",
        "import_preview_accounts",
        "import_preview_columns",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    assert_eq!(
        raw_byte_commands, reviewed,
        "a command takes or returns raw bytes: review that it cannot carry key material, \
         then add it to `reviewed`"
    );
}

#[test]
fn the_dek_scan_catches_what_it_should() {
    // Self-test of the matcher, so the contract above cannot silently pass on a typo.
    assert!(mentions_dek_word("let dek = x;").is_some());
    assert!(mentions_dek_word("fn f(dek: Dek)").is_some());
    assert!(mentions_dek_word("Dek([REDACTED])").is_some());
    assert!(mentions_dek_word("released_dek_bytes").is_some());
    assert!(mentions_dek_word("deck of cards").is_none());
    assert!(mentions_dek_word("index").is_none());
}

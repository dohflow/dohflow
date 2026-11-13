//! Derived identifiers (ADR 0013 Addendum A; personal-cfo-t2s4b): the known-answer
//! vectors are read from the ADR text itself, then determinism, ordering, the 4,096 cap
//! and input validation are checked. These are unit tests because the explicit-ordinal
//! functions they exercise are private to the crate on purpose.

use std::collections::{HashMap, HashSet};
use std::process::Command;

use uuid::Uuid;

use super::{
    derive_id, derive_id_traced, DerivedIdError, DerivedIdKey, DerivedIds, FileFingerprint,
    MAX_ORDINALS, MAX_PROVIDER_COMPONENT_LEN,
};
use crate::IssuedAt;

const ADR: &str = include_str!("../../../../docs/adr/0013-id-strategy.md");

/// The two normative vectors, parsed from the ADR's `vector A` / `vector B` blocks so the
/// test consumes the document's own numbers rather than a retyped copy.
fn adr_vectors() -> Vec<HashMap<String, String>> {
    let section = ADR
        .split_once("These two known-answer vectors are normative")
        .expect("the ADR has its vectors")
        .1;
    let fence = section
        .split("```")
        .nth(1)
        .expect("the vectors are in a code fence");
    let mut vectors: Vec<HashMap<String, String>> = Vec::new();
    for line in fence.lines() {
        let line = line.trim();
        if line.starts_with("vector ") {
            vectors.push(HashMap::new());
        } else if let Some((key, value)) = line.split_once('=') {
            vectors
                .last_mut()
                .expect("a vector header precedes its fields")
                .insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    vectors
}

fn unhex(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2), "{hex}");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn issued_ms(ms: i64) -> IssuedAt {
    IssuedAt::from_unix_micros(ms * 1_000)
}

#[test]
fn the_adr_vectors_reproduce_byte_for_byte() {
    let vectors = adr_vectors();
    assert_eq!(vectors.len(), 2, "vector A and vector B");
    for v in &vectors {
        let vault_id = Uuid::parse_str(&v["vault_id"]).unwrap();
        let command_id = Uuid::parse_str(&v["command_id"]).unwrap();
        let ms: i64 = v["issued_at_unix_ms"].parse().unwrap();
        let ordinal: u32 = v["ordinal"].parse().unwrap();

        let key = DerivedIdKey::for_vault(&vault_id);
        assert_eq!(key.as_bytes().as_slice(), unhex(&v["id_key"]), "id_key");

        let trace = derive_id_traced(&key, issued_ms(ms), &command_id, &v["tag"], ordinal).unwrap();
        assert_eq!(trace.message, unhex(&v["message_hex"]), "message");
        assert_eq!(trace.hmac.as_slice(), unhex(&v["hmac_sha256"]), "hmac");
        assert_eq!(
            trace.counter_random_bytes.as_slice(),
            unhex(&v["counter_random_bytes"]),
            "builder input"
        );
        assert_eq!(trace.uuid, Uuid::parse_str(&v["uuid"]).unwrap(), "uuid");
        assert_eq!(trace.uuid.get_version_num(), 7);
        assert_eq!(trace.uuid.get_variant(), uuid::Variant::RFC4122);
    }
}

fn sample_key() -> DerivedIdKey {
    DerivedIdKey::for_vault(&Uuid::from_u128(0x0011_2233_4455_6677_8899_aabb_ccdd_eeff))
}

fn sample_command() -> Uuid {
    Uuid::from_u128(0xffee_ddcc_bbaa_9988_7766_5544_3322_1100)
}

#[test]
fn every_input_changes_the_id() {
    let key = sample_key();
    let t = issued_ms(1_700_000_000_123);
    let c = sample_command();
    let base = derive_id(&key, t, &c, "ledger_transaction", 0).unwrap();

    assert_eq!(
        base,
        derive_id(&key, t, &c, "ledger_transaction", 0).unwrap()
    );
    assert_ne!(
        base,
        derive_id(&key, t, &c, "split_line", 0).unwrap(),
        "tag"
    );
    assert_ne!(
        base,
        derive_id(&key, t, &c, "ledger_transaction", 1).unwrap(),
        "ordinal"
    );
    assert_ne!(
        base,
        derive_id(&key, t, &Uuid::from_u128(1), "ledger_transaction", 0).unwrap(),
        "command"
    );
    assert_ne!(
        base,
        derive_id(
            &key,
            issued_ms(1_700_000_000_124),
            &c,
            "ledger_transaction",
            0
        )
        .unwrap(),
        "issued_at milliseconds"
    );
    let other_vault = DerivedIdKey::for_vault(&Uuid::from_u128(2));
    assert_ne!(
        base,
        derive_id(&other_vault, t, &c, "ledger_transaction", 0).unwrap(),
        "vault"
    );
}

#[test]
fn the_message_is_length_prefixed_so_tags_cannot_collide_across_the_ordinal() {
    // "a" + ordinal bytes 0x0001 versus "a\x00" would collide without a length prefix;
    // distinct tag lengths must give distinct messages.
    let key = sample_key();
    let t = issued_ms(1_700_000_000_123);
    let c = sample_command();
    let one = derive_id_traced(&key, t, &c, "ab", 1).unwrap();
    let two = derive_id_traced(&key, t, &c, "a", 0x6201).unwrap_err();
    assert!(matches!(two, DerivedIdError::OrdinalOverflow { .. }));
    assert_eq!(one.message.len(), 16 + 1 + 2 + 2);
    assert_eq!(one.message[16], 2, "the tag length is one octet");
}

#[test]
fn only_the_milliseconds_of_issued_at_matter() {
    let key = sample_key();
    let c = sample_command();
    let a = IssuedAt::from_unix_micros(1_700_000_000_123_000);
    let b = IssuedAt::from_unix_micros(1_700_000_000_123_999);
    assert_eq!(
        derive_id(&key, a, &c, "x", 0).unwrap(),
        derive_id(&key, b, &c, "x", 0).unwrap()
    );
}

#[test]
fn ids_sort_by_time_then_by_mint_order() {
    let key = sample_key();
    let c = sample_command();
    let t = issued_ms(1_700_000_000_123);

    // Within one command and one millisecond, ORDER BY id is mint order, over the
    // whole ordinal range, as bytes and as the hyphenated text a TEXT column stores.
    let ids: Vec<Uuid> = (0..MAX_ORDINALS)
        .map(|n| derive_id(&key, t, &c, "ledger_transaction", n).unwrap())
        .collect();
    assert!(ids.windows(2).all(|w| w[0].as_bytes() < w[1].as_bytes()));
    assert!(ids.windows(2).all(|w| w[0].to_string() < w[1].to_string()));

    // A later millisecond sorts after every id of an earlier one, whatever the ordinal.
    let later = derive_id(
        &key,
        issued_ms(1_700_000_000_124),
        &c,
        "ledger_transaction",
        0,
    )
    .unwrap();
    assert!(ids.iter().all(|id| id.as_bytes() < later.as_bytes()));
}

#[test]
fn the_cap_is_4096_ids_per_command() {
    let key = sample_key();
    let c = sample_command();
    let t = issued_ms(1_700_000_000_123);

    assert!(derive_id(&key, t, &c, "x", 4095).is_ok());
    let refused = derive_id(&key, t, &c, "x", 4096).unwrap_err();
    assert_eq!(
        refused,
        DerivedIdError::OrdinalOverflow {
            command_id: c,
            attempted: 4096
        }
    );

    let mut ids = DerivedIds::new(key, t, c);
    let mut seen = HashSet::new();
    for n in 0..4096 {
        assert_eq!(ids.minted(), n);
        assert!(
            seen.insert(ids.next("ledger_transaction").unwrap()),
            "ordinal {n}"
        );
    }
    assert_eq!((ids.minted(), ids.remaining()), (4096, 0));
    // The 4,097th fails, and keeps failing: no wrap, no reuse.
    for _ in 0..3 {
        assert_eq!(
            ids.next("ledger_transaction").unwrap_err(),
            DerivedIdError::OrdinalOverflow {
                command_id: c,
                attempted: 4096
            }
        );
    }
    assert_eq!(ids.minted(), 4096);
}

#[test]
fn the_allocator_mints_in_order_and_matches_derive_id() {
    let key = sample_key();
    let c = sample_command();
    let t = issued_ms(1_700_000_000_123);
    let mut ids = DerivedIds::new(key, t, c);
    let first = ids.next("account").unwrap();
    let second = ids.next("ledger_account").unwrap();
    assert_eq!(first, derive_id(&key, t, &c, "account", 0).unwrap());
    assert_eq!(second, derive_id(&key, t, &c, "ledger_account", 1).unwrap());
    // A failed call (a bad tag) consumes no ordinal.
    assert_eq!(ids.next("").unwrap_err(), DerivedIdError::EmptyTag);
    assert_eq!(ids.minted(), 2);
}

#[test]
fn tags_are_validated() {
    let key = sample_key();
    let c = sample_command();
    let t = issued_ms(1_700_000_000_123);
    let err = |tag: &str| derive_id(&key, t, &c, tag, 0).unwrap_err();

    assert_eq!(err(""), DerivedIdError::EmptyTag);
    assert_eq!(
        err(&"a".repeat(256)),
        DerivedIdError::TagTooLong { len: 256 }
    );
    assert_eq!(err("has space"), DerivedIdError::TagNotPrintableAscii);
    assert_eq!(err(" "), DerivedIdError::TagNotPrintableAscii);
    assert_eq!(err("tab\there"), DerivedIdError::TagNotPrintableAscii);
    assert_eq!(err("nul\0byte"), DerivedIdError::TagNotPrintableAscii);
    assert_eq!(err("del\u{7f}"), DerivedIdError::TagNotPrintableAscii);
    assert_eq!(err("caf\u{e9}"), DerivedIdError::TagNotPrintableAscii);
    assert_eq!(err("\u{1f4b0}"), DerivedIdError::TagNotPrintableAscii);

    // The boundaries are accepted: one byte, 255 bytes, and every printable character.
    assert!(derive_id(&key, t, &c, "a", 0).is_ok());
    assert!(derive_id(&key, t, &c, &"a".repeat(255), 0).is_ok());
    let printable: String = (0x21_u8..=0x7e).map(char::from).collect();
    assert!(derive_id(&key, t, &c, &printable, 0).is_ok());
}

#[test]
fn the_mint_accepts_exactly_the_supported_issued_at_range() {
    let key = sample_key();
    let c = sample_command();
    let at = IssuedAt::from_unix_micros;
    assert!(derive_id(&key, at(IssuedAt::MIN_SUPPORTED_MICROS), &c, "x", 0).is_ok());
    assert!(derive_id(&key, at(IssuedAt::MAX_SUPPORTED_MICROS), &c, "x", 0).is_ok());
    for micros in [
        IssuedAt::MIN_SUPPORTED_MICROS - 1,
        IssuedAt::MAX_SUPPORTED_MICROS + 1,
    ] {
        assert!(!at(micros).is_supported());
        assert_eq!(
            derive_id(&key, at(micros), &c, "x", 0).unwrap_err(),
            DerivedIdError::IssuedAtOutOfRange
        );
    }
}

#[test]
fn issued_at_must_fit_a_uuid_v7_timestamp() {
    let key = sample_key();
    let c = sample_command();
    let micros = |ms: i64| IssuedAt::from_unix_micros(ms * 1_000);
    assert_eq!(
        derive_id(&key, IssuedAt::from_unix_micros(-1_000), &c, "x", 0).unwrap_err(),
        DerivedIdError::IssuedAtOutOfRange
    );
    assert!(derive_id(&key, micros(0), &c, "x", 0).is_ok());
    // The 48-bit ceiling is far past the supported range (year 10889 against 9999), so the
    // supported range, not the timestamp width, is what the mint enforces.
    assert!(derive_id(&key, micros((1_i64 << 48) - 1), &c, "x", 0).is_err());
    assert_eq!(
        derive_id(&key, micros(1_i64 << 48), &c, "x", 0).unwrap_err(),
        DerivedIdError::IssuedAtOutOfRange
    );
}

#[test]
fn the_key_is_a_function_of_the_vault_id_only() {
    let vault = Uuid::from_u128(0x1122_3344_5566_7788_99aa_bbcc_ddee_ff00);
    assert_eq!(
        DerivedIdKey::for_vault(&vault),
        DerivedIdKey::for_vault(&vault)
    );
    assert_ne!(
        DerivedIdKey::for_vault(&vault),
        DerivedIdKey::for_vault(&Uuid::from_u128(1))
    );
    let bytes = *DerivedIdKey::for_vault(&vault).as_bytes();
    assert_eq!(
        DerivedIdKey::from_genesis(&vault, bytes).unwrap(),
        DerivedIdKey::for_vault(&vault)
    );
    assert_eq!(
        format!("{:?}", DerivedIdKey::for_vault(&vault)),
        "DerivedIdKey(..)"
    );
}

const CHILD_ENV: &str = "CORE_IDS_DERIVE_CHILD";

/// The id is a function of its inputs and nothing about the process: a second process
/// (a fresh address space, different clock reads, different thread ids) derives the same
/// value the parent does.
#[test]
fn a_second_process_derives_the_same_ids() {
    let key = sample_key();
    let c = sample_command();
    let t = issued_ms(1_700_000_000_123);
    let expected: Vec<String> = (0..16)
        .map(|n| {
            derive_id(&key, t, &c, "ledger_transaction", n)
                .unwrap()
                .to_string()
        })
        .collect();

    if std::env::var_os(CHILD_ENV).is_some() {
        // The child prints what it derived and the parent compares.
        for id in &expected {
            println!("DERIVED {id}");
        }
        return;
    }

    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "derived::tests::a_second_process_derives_the_same_ids",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let from_child: Vec<String> = stdout
        .lines()
        // libtest prints "test <name> ... " without a newline, so the first line is prefixed.
        .filter_map(|l| l.find("DERIVED ").map(|at| &l[at + "DERIVED ".len()..]))
        .map(str::to_owned)
        .collect();
    assert_eq!(from_child, expected);
}

#[test]
fn a_genesis_key_must_be_the_one_the_vault_id_derives() {
    let vault = Uuid::from_u128(0x1122_3344_5566_7788_99aa_bbcc_ddee_ff00);
    let other = Uuid::from_u128(7);
    let good = *DerivedIdKey::for_vault(&vault).as_bytes();

    assert!(DerivedIdKey::from_genesis(&vault, good).is_ok());
    // Another vault's key, an all-zero key and a one-bit flip are all refused.
    assert_eq!(
        DerivedIdKey::from_genesis(&vault, *DerivedIdKey::for_vault(&other).as_bytes())
            .unwrap_err(),
        DerivedIdError::GenesisKeyMismatch
    );
    assert_eq!(
        DerivedIdKey::from_genesis(&vault, [0; 32]).unwrap_err(),
        DerivedIdError::GenesisKeyMismatch
    );
    let mut flipped = good;
    flipped[31] ^= 1;
    assert_eq!(
        DerivedIdKey::from_genesis(&vault, flipped).unwrap_err(),
        DerivedIdError::GenesisKeyMismatch
    );
}

#[test]
fn two_allocators_for_the_same_command_agree_without_sharing_state() {
    // A replay builds a fresh allocator and mints the same sequence; the allocator has no
    // way to be copied mid-sequence (it is not `Clone`, checked by a doctest), so two
    // live allocators for one command can only be two deliberate constructions.
    let key = sample_key();
    let c = sample_command();
    let t = issued_ms(1_700_000_000_123);
    let mut original = DerivedIds::new(key, t, c);
    let mut replay = DerivedIds::new(key, t, c);
    for _ in 0..8 {
        assert_eq!(original.next("x").unwrap(), replay.next("x").unwrap());
    }
}

#[test]
fn a_keyed_id_is_a_pure_function_of_the_command_the_tag_and_the_length_prefixed_key() {
    // Known answers computed independently with Python's hmac/hashlib from the doc's construction.
    let key = sample_key();
    let command = sample_command();
    assert_eq!(
        key.keyed_id(&command, "scenario-event", &[&[1; 16], &[2; 16]])
            .unwrap()
            .to_string(),
        "76b3e713-7d89-815a-adc6-eea2f928a26c"
    );
    assert_eq!(
        key.keyed_id(&command, "reversal-txn", &[&[3; 16]])
            .unwrap()
            .to_string(),
        "77c9106d-a4c4-8e49-bd13-5acd6f1b7cb4"
    );
    // Parts are length-prefixed: the split point matters.
    let ab_c = key.keyed_id(&command, "x", &[b"ab", b"c"]).unwrap();
    let a_bc = key.keyed_id(&command, "x", &[b"a", b"bc"]).unwrap();
    assert_eq!(ab_c.to_string(), "7af329d4-98c6-8b41-8aef-db33dcf2bc67");
    assert_eq!(a_bc.to_string(), "b28cdcf4-9a13-8d2c-b36b-37ed12662488");
    assert_ne!(ab_c, a_bc);
    // The command is part of the key: another command mints another id for the same row.
    assert_eq!(
        key.keyed_id(&Uuid::from_u128(1), "reversal-txn", &[&[3; 16]])
            .unwrap()
            .to_string(),
        "ebcc5a57-6bf6-80be-8069-c97e7e6b79d8"
    );
    // Version 8, variant 10; the tag rules are the tag rules.
    let id = key.keyed_id(&command, "x", &[]).unwrap();
    assert_eq!(id.get_version_num(), 8);
    assert_eq!(id.as_bytes()[8] >> 6, 0b10);
    assert_eq!(
        key.keyed_id(&command, "", &[]).unwrap_err(),
        DerivedIdError::EmptyTag
    );
}

#[test]
fn a_keyed_mint_consumes_no_ordinal() {
    let mut ids = DerivedIds::new(sample_key(), issued_ms(1_700_000_000_123), sample_command());
    let first = ids.next("x").unwrap();
    ids.keyed("scenario-event", &[&[9; 16]]).unwrap();
    ids.keyed("scenario-event", &[&[8; 16]]).unwrap();
    assert_eq!(ids.minted(), 1);
    let mut replay = DerivedIds::new(sample_key(), issued_ms(1_700_000_000_123), sample_command());
    assert_eq!(replay.next("x").unwrap(), first);
}

#[test]
fn a_content_id_ignores_the_command_and_is_length_prefixed_with_known_answers() {
    // Known answers computed independently with Python's hmac/hashlib from the doc's construction.
    let key = sample_key();
    assert_eq!(
        key.content_id("balance-obs.connector", &[&[7; 16], b"2026-10-30"])
            .unwrap()
            .to_string(),
        "a4da6bf4-cd9b-85be-a9db-741a44cdb874"
    );
    let ab_c = key.content_id("x", &[b"ab", b"c"]).unwrap();
    let a_bc = key.content_id("x", &[b"a", b"bc"]).unwrap();
    assert_eq!(ab_c.to_string(), "c57d355e-9af5-8ec7-ad03-23a2bfc3706b");
    assert_eq!(a_bc.to_string(), "d153b378-5c22-8a77-9004-938e2b101e65");
    // Domain-separated from a keyed id over the same tag and key, whatever the command.
    let keyed = key
        .keyed_id(&sample_command(), "x", &[b"ab", b"c"])
        .unwrap();
    assert_eq!(keyed.to_string(), "7af329d4-98c6-8b41-8aef-db33dcf2bc67");
    assert_ne!(ab_c, keyed);
    // Version 8, variant 10; the tag rules are the tag rules.
    assert_eq!(ab_c.get_version_num(), 8);
    assert_eq!(ab_c.as_bytes()[8] >> 6, 0b10);
    assert_eq!(
        key.content_id("", &[]).unwrap_err(),
        DerivedIdError::EmptyTag
    );
}

#[test]
fn a_singleton_id_is_a_pure_function_of_the_vault_and_the_label_with_known_answers() {
    // Computed independently with Python's hmac/hashlib from the construction in the doc.
    let key = sample_key();
    assert_eq!(
        key.singleton_id("household").unwrap().to_string(),
        "d2d7ffd9-0631-8846-9da0-3c77affb2a3d"
    );
    assert_eq!(
        key.singleton_id("primary-member").unwrap().to_string(),
        "dacd5f62-1557-8e88-af8f-a201f33526b0"
    );
    // Version 8, variant 10.
    let id = key.singleton_id("household").unwrap();
    assert_eq!(id.get_version_num(), 8);
    assert_eq!(id.as_bytes()[8] >> 6, 0b10);
    // Another vault or another label is another id.
    let other = DerivedIdKey::for_vault(&Uuid::from_u128(1));
    assert_ne!(other.singleton_id("household").unwrap(), id);
    assert_ne!(key.singleton_id("primary-member").unwrap(), id);
    // The label rules are the tag rules.
    assert_eq!(key.singleton_id("").unwrap_err(), DerivedIdError::EmptyTag);
    assert_eq!(
        key.singleton_id("has space").unwrap_err(),
        DerivedIdError::TagNotPrintableAscii
    );
}

/// The known-answer blocks of ADR 0013 Addendum A section `number`, one map per blank-line-separated
/// vector, read from the document so the test consumes its own numbers rather than a retyped copy.
fn section_vectors(heading: &str, next: &str) -> Vec<HashMap<String, String>> {
    let section = ADR
        .split_once(heading)
        .unwrap_or_else(|| panic!("the ADR has {heading}"))
        .1
        .split_once(next)
        .unwrap_or_else(|| panic!("{next} follows {heading}"))
        .0;
    let block = section
        .split_once("Known-answer vector")
        .expect("the section has its vectors")
        .1
        .split("```text")
        .nth(1)
        .expect("the vectors are in a code fence")
        .split("```")
        .next()
        .expect("the fence closes");
    block
        .split("\n\n")
        .map(|chunk| {
            chunk
                .lines()
                .filter_map(|line| line.split_once('='))
                .map(|(k, v)| {
                    // A trailing note in parentheses is commentary, not part of the value.
                    let v = v.trim().split("(").next().unwrap_or("").trim();
                    (k.trim().to_owned(), v.to_owned())
                })
                .collect()
        })
        .collect()
}

/// The id key of the ADR vectors (vector A's vault) as the crate derives it, checked against the
/// literal the provider and file-row vectors name.
fn adr_vector_key(literal: &str) -> DerivedIdKey {
    let vector_a = &adr_vectors()[0];
    let key = DerivedIdKey::for_vault(&Uuid::parse_str(&vector_a["vault_id"]).unwrap());
    assert_eq!(key.as_bytes().to_vec(), unhex(literal), "vector A's key");
    key
}

fn hmac_hex(key: &DerivedIdKey, message: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(message);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_provider_vectors_of_section_9_reproduce_byte_for_byte() {
    let vectors = section_vectors(
        "#### 9. Exception: content-derived provider-transaction identifiers",
        "#### 10.",
    );
    assert_eq!(vectors.len(), 2);
    let key = adr_vector_key(&vectors[0]["id_key"]);
    let connection = Uuid::parse_str(&vectors[0]["connection_id"]).unwrap();
    let account = &vectors[0]["provider_account_key"];
    for (i, v) in vectors.iter().enumerate() {
        let txn = v
            .get("provider_txn_id")
            .unwrap_or(&vectors[0]["provider_txn_id"]);
        // The message, built here from the grammar rather than by the function under test.
        let mut message = vec![26_u8];
        message.extend_from_slice(b"dohflow/provider-txn-id/v1");
        message.extend_from_slice(connection.as_bytes());
        for part in [account, txn] {
            message.extend_from_slice(&u16::try_from(part.len()).unwrap().to_be_bytes());
            message.extend_from_slice(part.as_bytes());
        }
        if i == 0 {
            assert_eq!(hex(&message), v["message_hex"]);
            assert_eq!(hmac_hex(&key, &message), v["hmac_sha256"]);
        }
        let id = key.provider_txn_id(&connection, account, txn).unwrap();
        assert_eq!(id.to_string(), v["uuid"], "vector {i}");
        assert_eq!(id.get_version_num(), 8);
        assert_eq!(id.as_bytes()[8] >> 6, 0b10);
    }
}

#[test]
fn the_file_row_vectors_of_section_12_reproduce_byte_for_byte() {
    let vectors = section_vectors(
        "#### 12. Exception: file-row identifiers",
        "### Rejected alternatives",
    );
    assert_eq!(vectors.len(), 2);
    let key = adr_vector_key(&vectors[0]["id_key"]);
    let fingerprint = FileFingerprint::parse(&vectors[0]["file_fingerprint"]).unwrap();
    for (i, v) in vectors.iter().enumerate() {
        let ordinal: u32 = v["row_ordinal"].parse().unwrap();
        let mut message = vec![22_u8];
        message.extend_from_slice(b"dohflow/file-row-id/v1");
        message.extend_from_slice(fingerprint.as_bytes());
        message.extend_from_slice(&ordinal.to_be_bytes());
        assert_eq!(message.len(), 59, "a fixed 59 bytes");
        if i == 0 {
            assert_eq!(hex(&message), v["message_hex"]);
        }
        assert_eq!(hmac_hex(&key, &message), v["hmac_sha256"], "vector {i}");
        let id = key.file_row_id(&fingerprint, ordinal).unwrap();
        assert_eq!(id.to_string(), v["uuid"], "vector {i}");
        assert_eq!(id.get_version_num(), 8);
        assert_eq!(id.as_bytes()[8] >> 6, 0b10);
    }
}

#[test]
fn a_provider_id_depends_on_every_input_and_each_string_is_length_prefixed() {
    let key = sample_key();
    let connection = Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_2e3f_4a5b);
    let base = key.provider_txn_id(&connection, "ACT-1", "TXN-1").unwrap();
    assert_eq!(
        base,
        key.provider_txn_id(&connection, "ACT-1", "TXN-1").unwrap()
    );
    assert_ne!(
        base,
        key.provider_txn_id(&Uuid::from_u128(1), "ACT-1", "TXN-1")
            .unwrap()
    );
    assert_ne!(
        base,
        key.provider_txn_id(&connection, "ACT-2", "TXN-1").unwrap()
    );
    assert_ne!(
        base,
        key.provider_txn_id(&connection, "ACT-1", "TXN-2").unwrap()
    );
    let other_vault = DerivedIdKey::for_vault(&Uuid::from_u128(1));
    assert_ne!(
        base,
        other_vault
            .provider_txn_id(&connection, "ACT-1", "TXN-1")
            .unwrap()
    );
    // The length prefix keeps a boundary shift from colliding.
    assert_ne!(
        key.provider_txn_id(&connection, "ab", "c").unwrap(),
        key.provider_txn_id(&connection, "a", "bc").unwrap()
    );
    // Byte for byte: no case folding and no Unicode normalization.
    assert_ne!(
        base,
        key.provider_txn_id(&connection, "act-1", "TXN-1").unwrap()
    );
    assert_ne!(
        key.provider_txn_id(&connection, "caf\u{e9}", "t").unwrap(),
        key.provider_txn_id(&connection, "cafe\u{301}", "t")
            .unwrap()
    );
}

#[test]
fn a_provider_component_outside_the_byte_rules_has_no_identity() {
    let key = sample_key();
    let connection = Uuid::from_u128(7);
    let invalid = |component| DerivedIdError::ProviderComponentInvalid { component };
    // Empty, either side.
    assert_eq!(
        key.provider_txn_id(&connection, "", "t").unwrap_err(),
        invalid("account key")
    );
    assert_eq!(
        key.provider_txn_id(&connection, "a", "").unwrap_err(),
        invalid("transaction id")
    );
    // 1,024 bytes is the bound.
    let at_bound = "x".repeat(MAX_PROVIDER_COMPONENT_LEN);
    assert!(key
        .provider_txn_id(&connection, &at_bound, &at_bound)
        .is_ok());
    let over = "x".repeat(MAX_PROVIDER_COMPONENT_LEN + 1);
    assert_eq!(
        key.provider_txn_id(&connection, &over, "t").unwrap_err(),
        invalid("account key")
    );
    assert_eq!(
        key.provider_txn_id(&connection, "a", &over).unwrap_err(),
        invalid("transaction id")
    );
    // The bound is in BYTES: 513 two-byte characters are 1,026.
    let wide = "\u{e9}".repeat(513);
    assert_eq!(
        key.provider_txn_id(&connection, "a", &wide).unwrap_err(),
        invalid("transaction id")
    );
    // Every C0 control character and DEL, in the middle of an otherwise valid string.
    for byte in (0x00_u8..=0x1f).chain([0x7f]) {
        let bad = format!("a{}b", char::from(byte));
        assert_eq!(
            key.provider_txn_id(&connection, &bad, "t").unwrap_err(),
            invalid("account key"),
            "{byte:#04x}"
        );
        assert_eq!(
            key.provider_txn_id(&connection, "a", &bad).unwrap_err(),
            invalid("transaction id"),
            "{byte:#04x}"
        );
    }
    // A space, punctuation and non-ASCII text are legal.
    assert!(key
        .provider_txn_id(&connection, "a b/c-1", "t\u{e9}\u{4e2d}")
        .is_ok());
}

#[test]
fn a_file_fingerprint_is_exactly_sha256_and_sixty_four_lowercase_hex_characters() {
    let good = "sha256:2c30d6d4f9c6954bfc4f2e31460da5660e8d4aa9d893b6228eddc467516a6830";
    let parsed = FileFingerprint::parse(good).unwrap();
    assert_eq!(hex(parsed.as_bytes()), &good["sha256:".len()..]);
    assert_eq!(FileFingerprint::from_digest(*parsed.as_bytes()), parsed);
    let invalid = DerivedIdError::FileFingerprintInvalid;
    for bad in [
        "",
        "sha256:",
        "2c30d6d4f9c6954bfc4f2e31460da5660e8d4aa9d893b6228eddc467516a6830",
        &good.to_uppercase(),
        &good.replace("sha256:", "SHA256:"),
        &good.replace("sha256:", "sha512:"),
        &good[..good.len() - 1],
        &format!("{good}0"),
        &format!("{} ", good),
        &format!(" {good}"),
        &good.replace("2c30", "2C30"),
        &good.replace("2c30", "zc30"),
        &good.replace("sha256:", "sha256: "),
    ] {
        assert_eq!(FileFingerprint::parse(bad).unwrap_err(), invalid, "{bad:?}");
    }
    // The debug form does not print a digest into a log line.
    assert_eq!(format!("{parsed:?}"), "FileFingerprint(..)");
}

#[test]
fn a_file_row_id_depends_on_the_fingerprint_and_the_ordinal_and_counts_from_one() {
    let key = sample_key();
    let a = FileFingerprint::from_digest([1; 32]);
    let b = FileFingerprint::from_digest([2; 32]);
    let first = key.file_row_id(&a, 1).unwrap();
    assert_eq!(first, key.file_row_id(&a, 1).unwrap());
    assert_ne!(first, key.file_row_id(&a, 2).unwrap());
    assert_ne!(first, key.file_row_id(&b, 1).unwrap());
    assert_ne!(
        first,
        DerivedIdKey::for_vault(&Uuid::from_u128(1))
            .file_row_id(&a, 1)
            .unwrap()
    );
    // The ordinal is four bytes, so the top of the range is distinct and accepted.
    assert_ne!(
        key.file_row_id(&a, u32::MAX).unwrap(),
        key.file_row_id(&a, 255).unwrap()
    );
    assert_eq!(
        key.file_row_id(&a, 0).unwrap_err(),
        DerivedIdError::RowOrdinalZero
    );
}

#[test]
fn the_new_derivations_are_separate_from_every_other_one_under_the_same_key() {
    // The separation argument is in the ADR (sections 9 and 12); this is the behavioral check that
    // no two of the derivations here return one id for the inputs that could be confused.
    let key = sample_key();
    let fp = FileFingerprint::from_digest([9; 32]);
    let connection = Uuid::from_bytes([9; 16]);
    let ids = [
        key.file_row_id(&fp, 1).unwrap(),
        key.provider_txn_id(&connection, "a", "b").unwrap(),
        key.content_id("txn.import", &[&[9; 16], &1_u32.to_be_bytes()])
            .unwrap(),
        key.content_id("file-row-id", &[fp.as_bytes(), &1_u32.to_be_bytes()])
            .unwrap(),
        key.keyed_id(&connection, "file-row-id", &[fp.as_bytes()])
            .unwrap(),
        key.singleton_id("file-row-id").unwrap(),
    ];
    let distinct: HashSet<Uuid> = ids.iter().copied().collect();
    assert_eq!(distinct.len(), ids.len());
    // The message prefix bytes the ADR's argument rests on: 22 for a file row, 26 for a provider id.
    assert_ne!(22_u8, 26);
    assert_ne!(22_u8, b'd');
}

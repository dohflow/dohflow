//! The write-set hash assembly (vault-format.md C.4; personal-cfo-t2s4b part 5c). The vectors
//! were derived independently in Python (see the comments) and are pinned here; the
//! determinism properties are the rules the text states.

use sync_core::write_set::{write_set_hash, Entry, WriteSetError, MAX_ENTRIES};

fn entry(table: &str, key_hex: &str, fill: u8) -> Entry {
    Entry {
        table: table.to_owned(),
        primary_key: (0..key_hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&key_hex[i..i + 2], 16).unwrap())
            .collect(),
        digest: [fill; 32],
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn four() -> Vec<Entry> {
    vec![
        entry("tags", "030198c2c15f0a7c3d8a4e9b1d00000011", 0xAA),
        entry("accounts", "030198c2c15f0a7c3d8a4e9b1d00000001", 0xBB),
        entry("tags", "01ffffffffffffffff", 0xCC),
        entry("tag", "01fffffffffffffffe", 0xDD),
    ]
}

#[test]
fn the_known_answer_vectors_match_the_independent_derivation() {
    // Python: sort by (table bytes, key bytes); message = label ‖ u32 count ‖ per entry
    // u32 len ‖ table ‖ u32 len ‖ key ‖ digest; SHA-256. The shorter table name `tag` sorts
    // before `tags`, and within `tags` the int key (tag 01) sorts before the uuid key (tag 03).
    assert_eq!(
        hex(&write_set_hash(&mut four()).unwrap()),
        "01d58e040647aeea439caf5317f7843943e7b59cda0ae1e1ac246e2133f99a5e"
    );
    // The empty write set (a command that changed no replicated row) has a hash too.
    assert_eq!(
        hex(&write_set_hash(&mut []).unwrap()),
        "e1fc8564a73ed1395659bb939dfba9bfacd0adc2acfeaf2b52ead78f47e27b63"
    );
}

#[test]
fn the_hash_does_not_depend_on_the_order_entries_are_given_in() {
    let base = write_set_hash(&mut four()).unwrap();
    let mut reversed = four();
    reversed.reverse();
    assert_eq!(write_set_hash(&mut reversed).unwrap(), base);
    let mut rotated = four();
    rotated.rotate_left(2);
    assert_eq!(write_set_hash(&mut rotated).unwrap(), base);
}

#[test]
fn sorting_is_by_raw_key_bytes_not_by_length_prefixed_encodings() {
    // A shorter key that is a prefix of a longer one sorts first, and a longer key with a
    // smaller byte at the first difference sorts first regardless of length.
    let mut a = vec![
        entry("t", "0102", 1),
        entry("t", "01", 2),
        entry("t", "0003", 3),
    ];
    write_set_hash(&mut a).unwrap();
    let keys: Vec<String> = a.iter().map(|e| hex(&e.primary_key)).collect();
    assert_eq!(keys, ["0003", "01", "0102"]);
}

#[test]
fn any_change_to_an_entry_changes_the_hash() {
    let base = write_set_hash(&mut four()).unwrap();
    let mut digest = four();
    digest[0].digest[0] ^= 1;
    assert_ne!(write_set_hash(&mut digest).unwrap(), base);
    let mut key = four();
    key[1].primary_key[3] ^= 1;
    assert_ne!(write_set_hash(&mut key).unwrap(), base);
    let mut table = four();
    table[2].table = "tagz".to_owned();
    assert_ne!(write_set_hash(&mut table).unwrap(), base);
    let mut fewer = four();
    fewer.pop();
    assert_ne!(write_set_hash(&mut fewer).unwrap(), base);
}

#[test]
fn a_duplicate_row_and_invalid_entries_are_refused() {
    let mut dup = vec![entry("t", "01", 1), entry("t", "01", 2)];
    assert_eq!(write_set_hash(&mut dup), Err(WriteSetError::DuplicateEntry));
    assert_eq!(
        write_set_hash(&mut [entry("", "01", 1)]),
        Err(WriteSetError::BadEntry)
    );
    assert_eq!(
        write_set_hash(&mut [entry(&"n".repeat(65), "01", 1)]),
        Err(WriteSetError::BadEntry)
    );
    let long = "ab".repeat(257);
    assert_eq!(
        write_set_hash(&mut [entry("t", &long, 1)]),
        Err(WriteSetError::BadEntry)
    );
    assert_eq!(
        write_set_hash(&mut [entry("t", "", 1)]),
        Err(WriteSetError::BadEntry)
    );
}

#[test]
fn the_entry_count_is_capped() {
    let mut many: Vec<Entry> = (0..=MAX_ENTRIES)
        .map(|i| Entry {
            table: "t".to_owned(),
            primary_key: i.to_be_bytes().to_vec(),
            digest: [0; 32],
        })
        .collect();
    assert_eq!(
        write_set_hash(&mut many),
        Err(WriteSetError::TooManyEntries)
    );
}

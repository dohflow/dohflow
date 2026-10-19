//! The canonical row encoding (vault-format.md C.1.5; personal-cfo-t2s4b part 5b).
//!
//! The five known-answer vectors are read out of the specification and must be reproduced
//! byte for byte; the rest pins the rules the text states: tags follow the declared kind,
//! a mismatched storage class is an error and never coerced, columns sort by name bytes,
//! NULL/empty/zero are all distinct, the key cap is an error, and errors carry no values.

use sync_core::row_digest::{
    absent_digest, parent_row_digest, primary_key, row_digest, ChildTable, Column, Kind,
    RowDigestError, Value, MAX_PRIMARY_KEY,
};

const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");

fn vector(name: &str) -> Vec<u8> {
    let marker = format!("<!-- row-vector: {name}=");
    let hex = SPEC
        .split(&marker)
        .nth(1)
        .unwrap_or_else(|| panic!("missing vector {name}"))
        .split(" -->")
        .next()
        .unwrap();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

const TAG_ID: [u8; 16] = [
    0x01, 0x98, 0xc2, 0xc1, 0x5f, 0x0a, 0x7c, 0x3d, 0x8a, 0x4e, 0x9b, 0x1d, 0, 0, 0, 0x11,
];

fn spec_columns() -> Vec<Column<'static>> {
    vec![
        Column {
            name: "id",
            kind: Kind::Uuid,
            value: Value::Blob(&TAG_ID),
        },
        Column {
            name: "name",
            kind: Kind::Text,
            value: Value::Text("trip \u{e9}".as_bytes()),
        },
        Column {
            name: "color",
            kind: Kind::Text,
            value: Value::Null,
        },
        Column {
            name: "sort_order",
            kind: Kind::Int,
            value: Value::Int(-7),
        },
    ]
}

#[test]
fn the_specs_known_answer_vectors_are_reproduced() {
    let key = primary_key(&[(Kind::Uuid, Value::Blob(&TAG_ID))]).unwrap();
    assert_eq!(key, vector("primary_key"));
    assert_eq!(
        row_digest("tags", &spec_columns()).unwrap().to_vec(),
        vector("row_digest")
    );
    assert_eq!(
        absent_digest("tags", &key).unwrap().to_vec(),
        vector("absent_digest")
    );
}

#[test]
fn column_order_in_the_table_does_not_change_the_digest() {
    let forward = spec_columns();
    let mut reversed = spec_columns();
    reversed.reverse();
    assert_eq!(row_digest("tags", &forward), row_digest("tags", &reversed));
    // ... but the table name, a value, or a name does.
    let base = row_digest("tags", &forward).unwrap();
    assert_ne!(row_digest("tag", &forward).unwrap(), base);
    let mut changed = spec_columns();
    changed[3].value = Value::Int(-8);
    assert_ne!(row_digest("tags", &changed).unwrap(), base);
    let mut renamed = spec_columns();
    renamed[3].name = "sort_orders";
    assert_ne!(row_digest("tags", &renamed).unwrap(), base);
}

#[test]
fn null_empty_text_empty_bytes_and_zero_are_five_distinct_encodings() {
    let one = |kind, value| {
        row_digest(
            "t",
            &[Column {
                name: "c",
                kind,
                value,
            }],
        )
        .unwrap()
    };
    let digests = [
        one(Kind::Int, Value::Null),
        one(Kind::Int, Value::Int(0)),
        one(Kind::Text, Value::Text(b"")),
        one(Kind::Bytes, Value::Blob(b"")),
        one(Kind::Text, Value::Text(b"0")),
    ];
    for i in 0..digests.len() {
        for j in (i + 1)..digests.len() {
            // NULL under different kinds is the same single byte, so those are the only
            // allowed equalities; here every pair above uses a different tag or value.
            assert_ne!(digests[i], digests[j], "{i} vs {j}");
        }
    }
    // NULL is one byte for every kind.
    let null = |kind| {
        row_digest(
            "t",
            &[Column {
                name: "c",
                kind,
                value: Value::Null,
            }],
        )
        .unwrap()
    };
    assert_eq!(null(Kind::Int), null(Kind::Text));
    assert_eq!(null(Kind::Uuid), null(Kind::Bytes));
}

#[test]
fn the_declared_kind_decides_the_tag_and_a_mismatch_is_an_error_never_a_coercion() {
    let enc = |kind, value| {
        row_digest(
            "t",
            &[Column {
                name: "c",
                kind,
                value,
            }],
        )
    };
    // The same 16 bytes are a different encoding under uuid and bytes.
    assert_ne!(
        enc(Kind::Uuid, Value::Blob(&TAG_ID)).unwrap(),
        enc(Kind::Bytes, Value::Blob(&TAG_ID)).unwrap()
    );
    for (kind, value) in [
        (Kind::Int, Value::Text(b"7")),
        (Kind::Int, Value::Blob(&[7])),
        (Kind::Text, Value::Int(7)),
        (Kind::Text, Value::Blob(b"x")),
        (Kind::Uuid, Value::Text(&TAG_ID)),
        (Kind::Uuid, Value::Int(1)),
        (Kind::Bytes, Value::Text(b"x")),
        (Kind::Bytes, Value::Int(1)),
    ] {
        assert_eq!(
            enc(kind, value),
            Err(RowDigestError::KindMismatch),
            "{kind:?} {value:?}"
        );
    }
    assert_eq!(
        enc(Kind::Uuid, Value::Blob(&TAG_ID[..15])),
        Err(RowDigestError::BadUuidLength)
    );
    let seventeen = [TAG_ID.as_slice(), &[0]].concat();
    assert_eq!(
        enc(Kind::Uuid, Value::Blob(&seventeen)),
        Err(RowDigestError::BadUuidLength)
    );
    for bad in [&b"\xff"[..], b"\xc0\x80", b"\xed\xa0\x80", b"a\xe2\x82"] {
        assert_eq!(
            enc(Kind::Text, Value::Text(bad)),
            Err(RowDigestError::InvalidUtf8)
        );
    }
}

#[test]
fn values_and_names_are_bounded() {
    let big = vec![b'a'; (1 << 24) + 1];
    let ok = vec![b'a'; 1 << 24];
    let enc = |value| {
        row_digest(
            "t",
            &[Column {
                name: "c",
                kind: Kind::Text,
                value,
            }],
        )
    };
    assert!(enc(Value::Text(&ok)).is_ok());
    assert_eq!(enc(Value::Text(&big)), Err(RowDigestError::ValueTooLong));
    let long = "n".repeat(65);
    assert_eq!(
        row_digest(&long, &spec_columns()),
        Err(RowDigestError::BadName)
    );
    assert_eq!(
        row_digest("", &spec_columns()),
        Err(RowDigestError::BadName)
    );
    let mut named = spec_columns();
    named[0].name = &long;
    assert_eq!(row_digest("tags", &named), Err(RowDigestError::BadName));
    named[0].name = "color";
    assert_eq!(
        row_digest("tags", &named),
        Err(RowDigestError::DuplicateColumn)
    );
}

#[test]
fn a_primary_key_is_capped_at_256_bytes_and_never_truncated() {
    // text: tag(1) + length(4) + bytes, so 251 bytes of text is exactly 256.
    let fits = vec![b'k'; MAX_PRIMARY_KEY - 5];
    let over = vec![b'k'; MAX_PRIMARY_KEY - 4];
    assert_eq!(
        primary_key(&[(Kind::Text, Value::Text(&fits))])
            .unwrap()
            .len(),
        MAX_PRIMARY_KEY
    );
    assert_eq!(
        primary_key(&[(Kind::Text, Value::Text(&over))]),
        Err(RowDigestError::KeyTooLong)
    );
    // A composite key is capped as a whole.
    assert_eq!(
        primary_key(&[(Kind::Text, Value::Text(&fits)), (Kind::Int, Value::Int(1)),]),
        Err(RowDigestError::KeyTooLong)
    );
    assert_eq!(primary_key(&[]), Err(RowDigestError::EmptyKey));
    assert_eq!(
        absent_digest("t", &vec![0; MAX_PRIMARY_KEY + 1]),
        Err(RowDigestError::KeyTooLong)
    );
}

#[test]
fn composite_keys_do_not_collide_when_a_part_shifts() {
    let key = |a: &[u8], b: &[u8]| {
        primary_key(&[(Kind::Text, Value::Text(a)), (Kind::Text, Value::Text(b))]).unwrap()
    };
    assert_ne!(key(b"ab", b"c"), key(b"a", b"bc"));
    assert_ne!(key(b"", b"abc"), key(b"abc", b""));
    // An integer key and a text key spelling the same digits differ.
    assert_ne!(
        primary_key(&[(Kind::Int, Value::Int(7))]).unwrap(),
        primary_key(&[(Kind::Text, Value::Text(b"7"))]).unwrap()
    );
}

#[test]
fn the_row_and_deleted_row_digests_are_domain_separated() {
    let key = primary_key(&[(Kind::Uuid, Value::Blob(&TAG_ID))]).unwrap();
    assert_ne!(
        row_digest("tags", &spec_columns()).unwrap(),
        absent_digest("tags", &key).unwrap()
    );
    // A deletion is bound to its table and key.
    assert_ne!(
        absent_digest("tags", &key).unwrap(),
        absent_digest("tag", &key).unwrap()
    );
    let mut other = key.clone();
    other[5] ^= 1;
    assert_ne!(
        absent_digest("tags", &key).unwrap(),
        absent_digest("tags", &other).unwrap()
    );
}

#[test]
fn errors_never_carry_a_value_from_the_row() {
    let secret = "SECRET-4242.50";
    let err = row_digest(
        "t",
        &[Column {
            name: "c",
            kind: Kind::Int,
            value: Value::Text(secret.as_bytes()),
        }],
    )
    .unwrap_err();
    for text in [err.to_string(), format!("{err:?}")] {
        assert!(!text.contains("SECRET") && !text.contains("4242"), "{text}");
    }
}

// ---------------------------------------------------------------------------
// Folded children
// ---------------------------------------------------------------------------

const TXN: [u8; 16] = [
    0x01, 0x98, 0xc2, 0xc1, 0x5f, 0x0a, 0x7c, 0x3d, 0x8a, 0x4e, 0x9b, 0x1d, 0, 0, 0, 0x21,
];
const ACCT1: [u8; 16] = [
    0x01, 0x98, 0xc2, 0xc1, 0x5f, 0x0a, 0x7c, 0x3d, 0x8a, 0x4e, 0x9b, 0x1d, 0, 0, 0, 0x31,
];
const ACCT2: [u8; 16] = [
    0x01, 0x98, 0xc2, 0xc1, 0x5f, 0x0a, 0x7c, 0x3d, 0x8a, 0x4e, 0x9b, 0x1d, 0, 0, 0, 0x32,
];

fn posting<'a>(account: &'a [u8; 16], minor: i64) -> Vec<Column<'a>> {
    vec![
        Column {
            name: "transaction_id",
            kind: Kind::Uuid,
            value: Value::Blob(&TXN),
        },
        Column {
            name: "ledger_account_id",
            kind: Kind::Uuid,
            value: Value::Blob(account),
        },
        Column {
            name: "minor_units",
            kind: Kind::Int,
            value: Value::Int(minor),
        },
        Column {
            name: "currency",
            kind: Kind::Text,
            value: Value::Text(b"USD"),
        },
        Column {
            name: "currency_exponent",
            kind: Kind::Int,
            value: Value::Int(2),
        },
        Column {
            name: "posting_date",
            kind: Kind::Text,
            value: Value::Text(b"2026-10-17"),
        },
    ]
}

fn parent_columns() -> Vec<Column<'static>> {
    vec![
        Column {
            name: "id",
            kind: Kind::Uuid,
            value: Value::Blob(&TXN),
        },
        Column {
            name: "memo",
            kind: Kind::Text,
            value: Value::Text(b"split"),
        },
    ]
}

fn postings(rows: Vec<Vec<Column<'static>>>) -> Vec<ChildTable<'static>> {
    vec![ChildTable {
        table: "ledger_postings",
        rows,
    }]
}

#[test]
fn the_specs_parent_with_children_vector_is_reproduced() {
    let children = postings(vec![
        posting(&ACCT1, -500),
        posting(&ACCT2, 1000),
        posting(&ACCT1, -500),
    ]);
    assert_eq!(
        parent_row_digest("ledger_transactions", &parent_columns(), &children)
            .unwrap()
            .to_vec(),
        vector("parent_digest")
    );
}

#[test]
fn child_order_and_physical_row_order_do_not_matter_but_duplicates_do() {
    let base = parent_row_digest(
        "ledger_transactions",
        &parent_columns(),
        &postings(vec![
            posting(&ACCT1, -500),
            posting(&ACCT1, -500),
            posting(&ACCT2, 1000),
        ]),
    )
    .unwrap();
    let shuffled = parent_row_digest(
        "ledger_transactions",
        &parent_columns(),
        &postings(vec![
            posting(&ACCT2, 1000),
            posting(&ACCT1, -500),
            posting(&ACCT1, -500),
        ]),
    )
    .unwrap();
    assert_eq!(
        base, shuffled,
        "child order is the bytewise sort, not insertion order"
    );
    // Dropping one of the two identical postings changes the digest: a multiset.
    let fewer = parent_row_digest(
        "ledger_transactions",
        &parent_columns(),
        &postings(vec![posting(&ACCT1, -500), posting(&ACCT2, 1000)]),
    )
    .unwrap();
    assert_ne!(base, fewer);
    // So does changing any child value, or adding a child.
    let changed = parent_row_digest(
        "ledger_transactions",
        &parent_columns(),
        &postings(vec![
            posting(&ACCT1, -500),
            posting(&ACCT1, -501),
            posting(&ACCT2, 1000),
        ]),
    )
    .unwrap();
    assert_ne!(base, changed);
}

#[test]
fn a_parent_that_declares_children_is_not_confused_with_one_that_declares_none() {
    let plain = row_digest("ledger_transactions", &parent_columns()).unwrap();
    let empty_section =
        parent_row_digest("ledger_transactions", &parent_columns(), &postings(vec![])).unwrap();
    assert_ne!(
        plain, empty_section,
        "a declared child table with 0 rows differs from no section"
    );
}

#[test]
fn a_parent_with_no_child_tables_has_no_digest_because_the_spec_defines_none() {
    // C.1.5 has no section for undeclared children: a table that declares none is hashed with
    // row_digest, and one that declares children always lists them. An empty slice would be a
    // third form of the same state, so it is refused rather than hashed.
    assert_eq!(
        parent_row_digest("ledger_transactions", &parent_columns(), &[]),
        Err(RowDigestError::BadChildTables)
    );
}

#[test]
fn a_declared_child_table_with_zero_rows_has_a_known_answer() {
    // u16 count 1 || text(64) "ledger_postings" || u32 row count 0, after the parent's own row
    // message. The digest was derived independently (scripts/tests/row-digest-contract.test.sh
    // re-derives it from the layout and checks this literal is the one pinned here).
    let digest =
        parent_row_digest("ledger_transactions", &parent_columns(), &postings(vec![])).unwrap();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex,
        "f7e74df1401e4668ec28eef5310a30807eb613860bacd0fa44364a74f21dea9a"
    );
}

#[test]
fn child_tables_are_sorted_by_name_and_must_be_unique() {
    let a = ChildTable {
        table: "a_children",
        rows: vec![posting(&ACCT1, 1)],
    };
    let b = ChildTable {
        table: "b_children",
        rows: vec![posting(&ACCT2, 2)],
    };
    let one = parent_row_digest("p", &parent_columns(), &[a.clone(), b.clone()]).unwrap();
    let two = parent_row_digest("p", &parent_columns(), &[b.clone(), a.clone()]).unwrap();
    assert_eq!(one, two);
    assert_eq!(
        parent_row_digest("p", &parent_columns(), &[a.clone(), a]),
        Err(RowDigestError::BadChildTables)
    );
}

#[test]
fn a_child_row_over_the_cap_is_an_error() {
    let big = vec![b'x'; 70_000];
    let row = vec![Column {
        name: "c",
        kind: Kind::Text,
        value: Value::Text(&big),
    }];
    assert_eq!(
        parent_row_digest(
            "p",
            &parent_columns(),
            &[ChildTable {
                table: "k",
                rows: vec![row]
            }]
        ),
        Err(RowDigestError::ChildRowTooLong)
    );
}

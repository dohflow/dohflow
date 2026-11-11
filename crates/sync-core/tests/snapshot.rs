//! The snapshot plaintext (vault-format.md C.8, personal-cfo-1cltt.1): framing, manifest rules,
//! the state digest, and the typed refusal of each way a plaintext can be wrong.

use sha2::{Digest, Sha256};
use sync_core::canonical_json::to_canonical_vec;
use sync_core::cbe::Writer;
use sync_core::snapshot::{
    build, data_path, hex, verify, ColumnEntry, Header, Manifest, RecordedVoid, SnapshotError,
    StateDigest, TableFile, MANIFEST_PATH, MAX_PLAINTEXT, SNAPSHOT_LABEL,
};

fn columns() -> Vec<ColumnEntry> {
    vec![
        ColumnEntry {
            name: "id".into(),
            kind: "uuid".into(),
        },
        ColumnEntry {
            name: "name".into(),
            kind: "text".into(),
        },
    ]
}

fn table(name: &str, rows: &[&str]) -> TableFile {
    let content: Vec<u8> = rows
        .iter()
        .flat_map(|r| format!("{r}\n").into_bytes())
        .collect();
    TableFile {
        name: name.into(),
        columns: columns(),
        row_count: rows.len() as u64,
        state_digest: Some([7; 32]),
        content,
    }
}

fn header<'a>(voids: &'a [RecordedVoid], artifacts: &'a [[u8; 32]]) -> Header<'a> {
    Header {
        schema_version: 65,
        vault_created_at: "2026-01-02T03:04:05+00:00",
        inventory_version: "1",
        voids,
        artifacts,
    }
}

fn sample() -> Vec<u8> {
    build(
        &header(&[], &[]),
        vec![
            table("tags", &[r#"{"id":"a","name":"x"}"#]),
            table(
                "accounts",
                &[r#"{"id":"b","name":"y"}"#, r#"{"id":"c","name":"z"}"#],
            ),
            table("empty_table", &[]),
        ],
    )
    .expect("a sample snapshot")
}

/// Frame `(path, content)` pairs exactly as C.8 does, with no checks, so a test can build a
/// plaintext that `build` would refuse.
fn frame(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = Writer::with_label(SNAPSHOT_LABEL);
    w.u32(files.len() as u32);
    for (path, content) in files {
        w.text(255, path).unwrap();
        w.bytes(MAX_PLAINTEXT, content).unwrap();
    }
    w.finish()
}

fn manifest_of(plaintext: &[u8]) -> Manifest {
    verify(plaintext).expect("a valid snapshot").manifest
}

fn reframe(manifest: &Manifest, tables: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let manifest_bytes = to_canonical_vec(manifest).unwrap();
    let mut files: Vec<(&str, &[u8])> = vec![(MANIFEST_PATH, &manifest_bytes)];
    let paths: Vec<String> = tables.iter().map(|(n, _)| data_path(n)).collect();
    for (path, (_, content)) in paths.iter().zip(tables) {
        files.push((path.as_str(), content.as_slice()));
    }
    frame(&files)
}

fn files_of(plaintext: &[u8]) -> Vec<(String, Vec<u8>)> {
    verify(plaintext).unwrap().files
}

#[test]
fn a_built_snapshot_verifies_and_lists_its_tables_in_ascending_order() {
    let plaintext = sample();
    let verified = verify(&plaintext).unwrap();
    let names: Vec<&str> = verified
        .manifest
        .tables
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(names, ["accounts", "empty_table", "tags"]);
    assert_eq!(verified.manifest.tables[0].row_count, "2");
    assert_eq!(verified.manifest.tables[1].row_count, "0");
    assert_eq!(verified.manifest.schema_version, "65");
    assert_eq!(
        verified.manifest.vault_created_at,
        "2026-01-02T03:04:05+00:00"
    );
    // The framing: the label, a count, the manifest first, then the data files ascending.
    assert!(plaintext.starts_with(&{
        let mut w = Writer::with_label(SNAPSHOT_LABEL);
        w.u32(4);
        w.finish()
    }));
}

#[test]
fn building_is_deterministic_and_ignores_the_order_tables_are_handed_over() {
    let a = build(
        &header(&[], &[]),
        vec![table("b", &["{}"]), table("a", &["{}"])],
    )
    .unwrap();
    let b = build(
        &header(&[], &[]),
        vec![table("a", &["{}"]), table("b", &["{}"])],
    )
    .unwrap();
    assert_eq!(a, b);
}

#[test]
fn the_manifest_hash_is_the_sha256_of_the_file_and_an_empty_table_still_has_a_file() {
    let plaintext = sample();
    let verified = verify(&plaintext).unwrap();
    for (entry, (name, content)) in verified.manifest.tables.iter().zip(&verified.files) {
        assert_eq!(&entry.name, name);
        assert_eq!(entry.sha256, hex(&Sha256::digest(content)));
    }
    assert!(verified
        .files
        .iter()
        .any(|(n, c)| n == "empty_table" && c.is_empty()));
}

#[test]
fn a_folded_child_carries_no_state_digest_but_keeps_its_file() {
    let mut child = table("ledger_postings", &["{}"]);
    child.state_digest = None;
    let plaintext = build(&header(&[], &[]), vec![child]).unwrap();
    let manifest = manifest_of(&plaintext);
    assert_eq!(manifest.tables[0].state_digest, None);
    assert_eq!(files_of(&plaintext).len(), 1);
}

#[test]
fn voids_and_artifacts_are_recorded_in_order() {
    let voids = [
        RecordedVoid {
            void_id: [1; 32],
            first: 5,
            last: 9,
        },
        RecordedVoid {
            void_id: [2; 32],
            first: 10,
            last: 10,
        },
    ];
    let artifacts = [[3; 32], [4; 32]];
    let plaintext = build(&header(&voids, &artifacts), vec![table("a", &["{}"])]).unwrap();
    let manifest = manifest_of(&plaintext);
    assert_eq!(manifest.voids.len(), 2);
    assert_eq!(manifest.voids[0].first, "5");
    assert_eq!(manifest.voids[1].void_id, hex(&[2; 32]));
    assert_eq!(manifest.artifacts, [hex(&[3; 32]), hex(&[4; 32])]);
}

#[test]
fn voided_ranges_must_be_strictly_ascending_and_disjoint() {
    let overlapping = [
        RecordedVoid {
            void_id: [1; 32],
            first: 5,
            last: 9,
        },
        RecordedVoid {
            void_id: [2; 32],
            first: 9,
            last: 12,
        },
    ];
    let backwards = [RecordedVoid {
        void_id: [1; 32],
        first: 9,
        last: 5,
    }];
    let out_of_order = [
        RecordedVoid {
            void_id: [1; 32],
            first: 20,
            last: 21,
        },
        RecordedVoid {
            void_id: [2; 32],
            first: 5,
            last: 6,
        },
    ];
    for voids in [&overlapping[..], &backwards[..], &out_of_order[..]] {
        let result = build(&header(voids, &[]), vec![table("a", &["{}"])]);
        assert!(
            matches!(result, Err(SnapshotError::ManifestInvalid(_))),
            "{result:?}"
        );
    }
}

#[test]
fn the_closure_manifest_must_be_strictly_ascending() {
    for artifacts in [[[2u8; 32], [1; 32]], [[1; 32], [1; 32]]] {
        let result = build(&header(&[], &artifacts), vec![table("a", &["{}"])]);
        assert!(matches!(result, Err(SnapshotError::ManifestInvalid(_))));
    }
}

#[test]
fn at_most_1024_files_including_the_manifest() {
    let ok: Vec<TableFile> = (0..1023).map(|i| table(&format!("t{i:04}"), &[])).collect();
    assert!(build(&header(&[], &[]), ok).is_ok());
    let too_many: Vec<TableFile> = (0..1024).map(|i| table(&format!("t{i:04}"), &[])).collect();
    assert_eq!(
        build(&header(&[], &[]), too_many).unwrap_err(),
        SnapshotError::TooManyFiles
    );
}

#[test]
fn a_framing_that_claims_more_than_1024_files_is_refused() {
    let mut w = Writer::with_label(SNAPSHOT_LABEL);
    w.u32(1025);
    assert_eq!(
        verify(&w.finish()).unwrap_err(),
        SnapshotError::TooManyFiles
    );
    let mut w = Writer::with_label(SNAPSHOT_LABEL);
    w.u32(0);
    assert_eq!(
        verify(&w.finish()).unwrap_err(),
        SnapshotError::TooManyFiles
    );
}

#[test]
fn a_plaintext_over_one_gibibyte_is_snapshot_too_large() {
    // Zeroed allocations are not touched, so this costs address space, not memory.
    let big = vec![0u8; MAX_PLAINTEXT + 1];
    assert_eq!(verify(&big).unwrap_err(), SnapshotError::TooLarge);
    let mut file = table("a", &[]);
    file.content = vec![0u8; MAX_PLAINTEXT + 1];
    assert_eq!(
        build(&header(&[], &[]), vec![file]).unwrap_err(),
        SnapshotError::TooLarge
    );
}

#[test]
fn the_first_file_must_be_the_manifest() {
    let plaintext = frame(&[("data/a.jsonl", b"")]);
    assert_eq!(
        verify(&plaintext).unwrap_err(),
        SnapshotError::FirstFileNotManifest
    );
}

#[test]
fn data_files_must_be_in_strictly_ascending_path_order() {
    let good = sample();
    let manifest = manifest_of(&good);
    let files = files_of(&good);
    let mut reversed: Vec<(&str, Vec<u8>)> =
        files.iter().map(|(n, c)| (n.as_str(), c.clone())).collect();
    reversed.reverse();
    assert_eq!(
        verify(&reframe(&manifest, &reversed)).unwrap_err(),
        SnapshotError::PathsNotAscending
    );
    let duplicated = [reversed[0].clone(), reversed[0].clone()];
    assert_eq!(
        verify(&reframe(&manifest, &duplicated)).unwrap_err(),
        SnapshotError::PathsNotAscending
    );
}

#[test]
fn a_path_that_is_not_a_data_file_is_refused() {
    let manifest = manifest_of(&sample());
    let manifest_bytes = to_canonical_vec(&manifest).unwrap();
    for path in [
        "data/a.json",
        "x/a.jsonl",
        "data/A.jsonl",
        "data/.jsonl",
        "data/a/b.jsonl",
        "data/a b.jsonl",
    ] {
        let plaintext = frame(&[(MANIFEST_PATH, &manifest_bytes), (path, b"")]);
        assert_eq!(
            verify(&plaintext).unwrap_err(),
            SnapshotError::BadPath,
            "{path}"
        );
    }
}

#[test]
fn trailing_and_truncated_bytes_are_refused() {
    let mut plaintext = sample();
    plaintext.push(0);
    assert!(matches!(verify(&plaintext), Err(SnapshotError::Framing(_))));
    let mut plaintext = sample();
    plaintext.pop();
    assert!(matches!(verify(&plaintext), Err(SnapshotError::Framing(_))));
    assert!(matches!(
        verify(b"dohflow/other/v1"),
        Err(SnapshotError::Framing(_))
    ));
}

#[test]
fn a_missing_or_an_extra_file_is_a_file_set_mismatch() {
    let good = sample();
    let manifest = manifest_of(&good);
    let files = files_of(&good);
    let all: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, c)| (n.as_str(), c.clone())).collect();
    assert_eq!(
        verify(&reframe(&manifest, &all[1..])).unwrap_err(),
        SnapshotError::FileSetMismatch
    );
    let mut extra = all.clone();
    extra.push(("zzz", Vec::new()));
    assert_eq!(
        verify(&reframe(&manifest, &extra)).unwrap_err(),
        SnapshotError::FileSetMismatch
    );
}

#[test]
fn a_changed_file_fails_its_hash_and_a_wrong_line_count_fails_its_count() {
    let good = sample();
    let manifest = manifest_of(&good);
    let files = files_of(&good);
    let mut tampered: Vec<(&str, Vec<u8>)> =
        files.iter().map(|(n, c)| (n.as_str(), c.clone())).collect();
    tampered[0].1[3] ^= 1;
    assert_eq!(
        verify(&reframe(&manifest, &tampered)).unwrap_err(),
        SnapshotError::HashMismatch
    );

    // A manifest whose hash is right but whose row count is not.
    let mut lying = manifest.clone();
    lying.tables[0].row_count = "5".into();
    let intact: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, c)| (n.as_str(), c.clone())).collect();
    assert_eq!(
        verify(&reframe(&lying, &intact)).unwrap_err(),
        SnapshotError::RowCountMismatch
    );

    // A file with no final newline.
    let mut unterminated = manifest.clone();
    let mut content = files[0].1.clone();
    content.pop();
    unterminated.tables[0].sha256 = hex(&Sha256::digest(&content));
    unterminated.tables[0].row_count = "1".into();
    let mut parts: Vec<(&str, Vec<u8>)> =
        files.iter().map(|(n, c)| (n.as_str(), c.clone())).collect();
    parts[0].1 = content;
    assert_eq!(
        verify(&reframe(&unterminated, &parts)).unwrap_err(),
        SnapshotError::RowCountMismatch
    );
}

#[test]
fn a_manifest_that_is_not_canonical_json_is_refused() {
    let good = sample();
    let manifest = manifest_of(&good);
    let files = files_of(&good);
    let parts: Vec<(String, Vec<u8>)> = files;
    let canonical = to_canonical_vec(&manifest).unwrap();

    // Whitespace is not the canonical spelling.
    let mut spaced = Vec::new();
    spaced.push(b' ');
    spaced.extend_from_slice(&canonical);
    let paths: Vec<String> = parts.iter().map(|(n, _)| data_path(n)).collect();
    let mut framed: Vec<(&str, &[u8])> = vec![(MANIFEST_PATH, &spaced)];
    for (path, (_, content)) in paths.iter().zip(&parts) {
        framed.push((path.as_str(), content.as_slice()));
    }
    assert!(matches!(
        verify(&frame(&framed)),
        Err(SnapshotError::ManifestNotCanonical(_))
    ));

    // An unknown member is refused, not ignored.
    let text = String::from_utf8(canonical.clone()).unwrap();
    let with_extra = text.replacen('{', r#"{"extra":"1","#, 1);
    // Member order is canonical order, so this is refused as non-canonical or unknown either way.
    let mut framed: Vec<(&str, &[u8])> = vec![(MANIFEST_PATH, with_extra.as_bytes())];
    for (path, (_, content)) in paths.iter().zip(&parts) {
        framed.push((path.as_str(), content.as_slice()));
    }
    assert!(matches!(
        verify(&frame(&framed)),
        Err(SnapshotError::ManifestNotCanonical(_))
    ));
}

#[test]
fn manifest_rules_each_have_a_typed_refusal() {
    let good = sample();
    let base = manifest_of(&good);
    let files = files_of(&good);
    let parts: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, c)| (n.as_str(), c.clone())).collect();
    let bad = |edit: &dyn Fn(&mut Manifest)| {
        let mut manifest = base.clone();
        edit(&mut manifest);
        verify(&reframe(&manifest, &parts)).unwrap_err()
    };
    let invalid = |e: SnapshotError| matches!(e, SnapshotError::ManifestInvalid(_));
    assert!(invalid(bad(&|m| m.format = "other".into())));
    assert!(invalid(bad(&|m| m.format_version = "2".into())));
    assert!(invalid(bad(&|m| m.snapshot_schema_version = "2".into())));
    assert!(invalid(bad(&|m| m.schema_version = "065".into())));
    assert!(invalid(bad(&|m| m.schema_version = "-1".into())));
    assert!(invalid(bad(&|m| m.vault_created_at = String::new())));
    assert!(invalid(bad(&|m| m.tables[0].row_count = "01".into())));
    assert!(invalid(bad(&|m| m.tables[0].sha256 = "AB".repeat(32))));
    assert!(invalid(bad(&|m| m.tables[0].sha256 = "ab".repeat(31))));
    assert!(invalid(bad(
        &|m| m.tables[0].state_digest = Some("zz".repeat(32))
    )));
    assert!(invalid(bad(
        &|m| m.tables[0].columns[0].kind = "float".into()
    )));
    assert!(invalid(bad(&|m| m.tables[0].columns.clear())));
    assert!(invalid(bad(&|m| m.tables[0].name = "Bad-Name".into())));
    assert!(invalid(bad(&|m| m.tables.swap(0, 1))));
    assert!(invalid(bad(&|m| m.artifacts = vec!["xyz".into()])));
}

// ---------------------------------------------------------------------------------------------
// The state digest

#[test]
fn the_state_digest_is_the_label_table_count_and_the_row_digests_in_key_order() {
    let mut digest = StateDigest::new("accounts").unwrap();
    digest.push(&[1], [0xaa; 32]).unwrap();
    digest.push(&[1, 0], [0xbb; 32]).unwrap();
    digest.push(&[2], [0xcc; 32]).unwrap();
    let got = digest.finish().unwrap();

    let mut w = Writer::with_label("dohflow/sync-state-digest/v1");
    w.text(64, "accounts").unwrap();
    w.u64(3);
    w.fixed(&[0xaa; 32]).fixed(&[0xbb; 32]).fixed(&[0xcc; 32]);
    let want: [u8; 32] = Sha256::digest(w.as_bytes()).into();
    assert_eq!(got, want);
}

#[test]
fn an_empty_table_has_a_digest_and_it_depends_on_the_table_name() {
    let empty = |name: &str| StateDigest::new(name).unwrap().finish().unwrap();
    assert_eq!(empty("accounts"), empty("accounts"));
    assert_ne!(empty("accounts"), empty("categories"));
}

#[test]
fn the_state_digest_refuses_rows_out_of_order_or_repeated() {
    let mut digest = StateDigest::new("t").unwrap();
    digest.push(&[2], [1; 32]).unwrap();
    for key in [&[1u8][..], &[2][..]] {
        assert!(matches!(
            digest.push(key, [1; 32]),
            Err(SnapshotError::ManifestInvalid(_))
        ));
    }
    // A prefix sorts first: [1] before [1, 0].
    let mut digest = StateDigest::new("t").unwrap();
    digest.push(&[1, 0], [1; 32]).unwrap();
    assert!(digest.push(&[1], [1; 32]).is_err());
}

#[test]
fn a_changed_row_digest_a_missing_row_or_a_renamed_table_changes_the_state_digest() {
    let run = |table: &str, rows: &[(&[u8], [u8; 32])]| {
        let mut digest = StateDigest::new(table).unwrap();
        for (key, row) in rows {
            digest.push(key, *row).unwrap();
        }
        digest.finish().unwrap()
    };
    let base = run("t", &[(&[1], [1; 32]), (&[2], [2; 32])]);
    assert_ne!(base, run("t", &[(&[1], [1; 32]), (&[2], [3; 32])]));
    assert_ne!(base, run("t", &[(&[1], [1; 32])]));
    assert_ne!(base, run("u", &[(&[1], [1; 32]), (&[2], [2; 32])]));
    // The key itself is not part of the digest: only the row digests, in key order.
    assert_eq!(base, run("t", &[(&[9], [1; 32]), (&[10], [2; 32])]));
}

#[test]
fn a_bad_table_name_is_refused() {
    for name in ["", "Accounts", "a-b", "a b", &"a".repeat(65)] {
        assert!(StateDigest::new(name).is_err(), "{name}");
    }
}

// ---------------------------------------------------------------------------------------------
// Rows

mod rows {
    use sync_core::row_digest::{child_row_bytes, Column, Kind, Value};
    use sync_core::snapshot::{base64, row_line, uuid_text, SnapshotError};

    fn col<'a>(name: &'a str, kind: Kind, value: Value<'a>) -> Column<'a> {
        Column { name, kind, value }
    }

    #[test]
    fn a_row_is_a_canonical_object_with_each_kind_in_its_b3_encoding() {
        let id = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ];
        let line = row_line(&[
            col("name", Kind::Text, Value::Text("Rent \"main\"".as_bytes())),
            col("amount", Kind::Int, Value::Int(-1250)),
            col("id", Kind::Uuid, Value::Blob(&id)),
            col("blob", Kind::Bytes, Value::Blob(&[0xff, 0x00, 0x10])),
            col("note", Kind::Text, Value::Null),
        ])
        .unwrap();
        assert_eq!(
            String::from_utf8(line).unwrap(),
            "{\"amount\":\"-1250\",\"blob\":\"/wAQ\",\"id\":\"01234567-89ab-cdef-0123-456789abcdef\",\"name\":\"Rent \\\"main\\\"\",\"note\":null}\n"
        );
    }

    #[test]
    fn a_null_is_null_in_every_kind() {
        for kind in [Kind::Int, Kind::Text, Kind::Uuid, Kind::Bytes] {
            let line = row_line(&[col("v", kind, Value::Null)]).unwrap();
            assert_eq!(line, b"{\"v\":null}\n");
        }
    }

    #[test]
    fn a_storage_class_that_does_not_match_the_kind_is_refused_naming_the_column() {
        let cases: Vec<(Kind, Value<'_>)> = vec![
            (Kind::Int, Value::Text(b"5")),
            (Kind::Text, Value::Int(5)),
            (Kind::Text, Value::Blob(b"x")),
            (Kind::Text, Value::Text(&[0xff, 0xfe])),
            (Kind::Uuid, Value::Blob(&[1; 15])),
            (Kind::Uuid, Value::Blob(&[1; 17])),
            (Kind::Uuid, Value::Text(b"x")),
            (Kind::Bytes, Value::Int(1)),
            (Kind::Bytes, Value::Text(b"x")),
        ];
        for (kind, value) in cases {
            let error = row_line(&[col("the_column", kind, value)]).unwrap_err();
            assert_eq!(
                error,
                SnapshotError::KindMismatch {
                    column: "the_column".into()
                },
                "{kind:?} {value:?}"
            );
            // The refusal names the column and never a stored value.
            assert!(!error.to_string().contains("xfe"));
        }
    }

    #[test]
    fn text_with_a_nul_cannot_be_written() {
        let error = row_line(&[col("t", Kind::Text, Value::Text(b"a\0b"))]).unwrap_err();
        assert_eq!(error, SnapshotError::Unrepresentable { column: "t".into() });
    }

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), want, "{input}");
        }
    }

    #[test]
    fn a_uuid_is_lowercase_hyphenated() {
        assert_eq!(
            uuid_text([0xAB; 16]),
            "abababab-abab-abab-abab-abababababab"
        );
    }

    #[test]
    fn the_folded_child_row_bytes_sort_columns_by_name() {
        let a = child_row_bytes(&[
            col("b", Kind::Int, Value::Int(1)),
            col("a", Kind::Int, Value::Int(2)),
        ])
        .unwrap();
        let b = child_row_bytes(&[
            col("a", Kind::Int, Value::Int(2)),
            col("b", Kind::Int, Value::Int(1)),
        ])
        .unwrap();
        assert_eq!(a, b);
        // u16 count, then name-sorted (text(64) name, tag, i64) pairs.
        assert_eq!(&a[..2], &[0, 2]);
    }
}

// ---- reading rows back: parse_row_line is the exact inverse of row_line --------------------------

mod reading_rows {
    use sync_core::row_digest::{Column, Kind, Value};
    use sync_core::snapshot::{parse_row_line, row_line, ColumnEntry, OwnedValue, SnapshotError};

    fn entries() -> Vec<ColumnEntry> {
        ["a_int", "b_text", "c_uuid", "d_bytes"]
            .into_iter()
            .zip(["int", "text", "uuid", "bytes"])
            .map(|(name, kind)| ColumnEntry {
                name: name.to_owned(),
                kind: kind.to_owned(),
            })
            .collect()
    }

    fn line(int: Value<'_>, text: Value<'_>, uuid: Value<'_>, bytes: Value<'_>) -> Vec<u8> {
        row_line(&[
            Column {
                name: "a_int",
                kind: Kind::Int,
                value: int,
            },
            Column {
                name: "b_text",
                kind: Kind::Text,
                value: text,
            },
            Column {
                name: "c_uuid",
                kind: Kind::Uuid,
                value: uuid,
            },
            Column {
                name: "d_bytes",
                kind: Kind::Bytes,
                value: bytes,
            },
        ])
        .unwrap()
    }

    #[test]
    fn every_kind_round_trips_including_null_padding_and_awkward_text() {
        let uuid = [0xab; 16];
        for bytes in [
            &b""[..],
            b"a",
            b"ab",
            b"abc",
            b"abcd",
            &[0, 255, 254, 1, 0, 7],
        ] {
            for text in [
                "",
                "plain",
                "quote \" backslash \\ slash / tab \t nl \n",
                "héllo ✓ 𝄞",
            ] {
                for int in [0_i64, 1, -1, 42, i64::MAX, i64::MIN] {
                    let encoded = line(
                        Value::Int(int),
                        Value::Text(text.as_bytes()),
                        Value::Blob(&uuid),
                        Value::Blob(bytes),
                    );
                    assert_eq!(
                        parse_row_line(&encoded, &entries()).unwrap(),
                        vec![
                            OwnedValue::Int(int),
                            OwnedValue::Text(text.to_owned()),
                            OwnedValue::Blob(uuid.to_vec()),
                            OwnedValue::Blob(bytes.to_vec()),
                        ]
                    );
                }
            }
        }
        // NULL is null in any kind.
        let encoded = line(Value::Null, Value::Null, Value::Null, Value::Null);
        assert_eq!(
            parse_row_line(&encoded, &entries()).unwrap(),
            vec![OwnedValue::Null; 4]
        );
    }

    #[test]
    fn a_line_that_is_not_the_canonical_encoding_of_its_columns_is_refused() {
        let good = line(
            Value::Int(5),
            Value::Text(b"x"),
            Value::Blob(&[1; 16]),
            Value::Blob(b"ab"),
        );
        let text = String::from_utf8(good.clone()).unwrap();
        let malformed = |bytes: &[u8]| {
            assert_eq!(
                parse_row_line(bytes, &entries()).unwrap_err(),
                SnapshotError::MalformedRow,
                "{}",
                String::from_utf8_lossy(bytes)
            );
        };
        // No newline, two newlines, whitespace, and a non-canonical key order.
        malformed(text.trim_end().as_bytes());
        malformed(format!("{text}\n").as_bytes());
        malformed(text.replace("\":\"", "\": \"").as_bytes());
        malformed(b"[]\n");
        malformed(b"null\n");
        malformed(b"\n");
        // A missing column and an extra one.
        let one_fewer = &entries()[..3];
        assert_eq!(
            parse_row_line(&good, one_fewer).unwrap_err(),
            SnapshotError::MalformedRow
        );
        let mut one_more = entries();
        one_more.push(ColumnEntry {
            name: "e_more".to_owned(),
            kind: "int".to_owned(),
        });
        assert_eq!(
            parse_row_line(&good, &one_more).unwrap_err(),
            SnapshotError::MalformedRow
        );
    }

    #[test]
    fn a_value_in_any_spelling_but_the_canonical_one_is_refused_naming_its_column() {
        // One column at a time, each value a non-canonical spelling of its kind.
        let refused = |kind: &str, json_value: &str| {
            let columns = [ColumnEntry {
                name: "v".to_owned(),
                kind: kind.to_owned(),
            }];
            let line = format!("{{\"v\":{json_value}}}\n");
            assert_eq!(
                parse_row_line(line.as_bytes(), &columns).unwrap_err(),
                SnapshotError::KindMismatch {
                    column: "v".to_owned()
                },
                "{kind} {json_value}"
            );
        };
        for bad in [
            "\"+5\"",
            "\"05\"",
            "\"-0\"",
            "\"5.0\"",
            "\"1e3\"",
            "\"\"",
            "\"9223372036854775808\"",
            "true",
            "[]",
        ] {
            refused("int", bad);
        }
        for bad in [
            "\"ABCDEFAB-CDEF-ABCD-EFAB-CDEFABCDEFAB\"",
            "\"abcdefabcdefabcdefabcdefabcdefab\"",
            "\"abcdefab-cdef-abcd-efab-cdefabcdefa\"",
            "\"abcdefab-cdef-abcd-efab-cdefabcdefabc\"",
            "\"gbcdefab-cdef-abcd-efab-cdefabcdefab\"",
        ] {
            refused("uuid", bad);
        }
        for bad in [
            "\"YWI\"",     // unpadded
            "\"YWI=\\n\"", // whitespace
            "\"YW=I\"",    // padding in the middle
            "\"YR==\"",    // non-zero trailing bits (not the canonical spelling of any byte string)
            "\"YWJj====\"",
            "\"!!!!\"",
        ] {
            refused("bytes", bad);
        }
        refused("text", "true");
        // A bare number is not canonical JSON in this subset at all.
        for kind in ["int", "uuid", "bytes", "text"] {
            let columns = [ColumnEntry {
                name: "v".to_owned(),
                kind: kind.to_owned(),
            }];
            assert_eq!(
                parse_row_line(b"{\"v\":5}\n", &columns).unwrap_err(),
                SnapshotError::MalformedRow,
                "{kind}"
            );
        }
        refused("float", "\"1\"");
        // U+0000 in any string is refused by the canonical parser: such a row cannot exist.
        let columns = [ColumnEntry {
            name: "v".to_owned(),
            kind: "text".to_owned(),
        }];
        assert_eq!(
            parse_row_line(b"{\"v\":\"a\\u0000b\"}\n", &columns).unwrap_err(),
            SnapshotError::MalformedRow
        );
    }
}

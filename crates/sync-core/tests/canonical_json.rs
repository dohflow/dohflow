//! The canonical JSON codec (personal-cfo-t2s4b part 3a; vault-format.md C.1.2).
//!
//! Three layers of evidence:
//!
//! 1. **The pinned crate against an independent oracle.** `oracle` below is a hand-written
//!    canonicalizer for the restricted profile (about 40 lines, test only). The crate must
//!    agree with it on the RFC 8785 examples, on the spec's own manifest vector and on a
//!    deterministic generated corpus, so a regression in the crate, or a bump of its
//!    pinned version, shows up as a failing test.
//! 2. **The strictness the crate does not provide.** No JSON number, no float, no
//!    duplicate member, no lone surrogate, no invalid UTF-8, ASCII member names,
//!    decimal-string integers, canonical-or-reject decoding.
//! 3. **The parser against `serde_json`** as a differential check.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sync_core::canonical_json::{
    from_canonical_slice, node_to_canonical_vec, parse_canonical_node, parse_node_lenient,
    to_canonical_vec, CanonicalJsonError, Node, MAX_DEPTH,
};

// ---------------------------------------------------------------------------
// The oracle: an independent canonicalizer for the restricted profile
// ---------------------------------------------------------------------------

fn oracle_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Member names sort by UTF-16 code units, as RFC 8785 section 3.2.3 requires.
fn oracle(node: &Node) -> String {
    match node {
        Node::Null => "null".to_owned(),
        Node::Bool(b) => b.to_string(),
        Node::String(s) => oracle_string(s),
        Node::Array(items) => format!(
            "[{}]",
            items.iter().map(oracle).collect::<Vec<_>>().join(",")
        ),
        Node::Object(members) => {
            let mut entries: Vec<(&String, &Node)> = members.iter().collect();
            entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            let body: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!("{}:{}", oracle_string(k), oracle(v)))
                .collect();
            format!("{{{}}}", body.join(","))
        }
    }
}

fn obj(entries: &[(&str, Node)]) -> Node {
    Node::Object(
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    )
}

fn s(text: &str) -> Node {
    Node::String(text.to_owned())
}

// ---------------------------------------------------------------------------
// A deterministic generated corpus
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % (n as u64)).unwrap()
    }
}

/// Characters chosen to hit every escaping and ordering edge: quotes, backslash, solidus,
/// every control range, DEL, line separators, the BMP edge, an astral scalar, combining
/// marks, and ordinary text.
const POOL: &[char] = &[
    'a',
    'Z',
    '0',
    ' ',
    '"',
    '\\',
    '/',
    '\u{1}',
    '\u{8}',
    '\u{9}',
    '\u{a}',
    '\u{c}',
    '\u{d}',
    '\u{1f}',
    '\u{7f}',
    '\u{80}',
    'é',
    '€',
    '\u{2028}',
    '\u{2029}',
    '\u{feff}',
    '\u{ffff}',
    '\u{10000}',
    '\u{1f600}',
    '\u{301}',
    '~',
    '\'',
];

fn random_string(rng: &mut Rng) -> String {
    (0..rng.below(9))
        .map(|_| POOL[rng.below(POOL.len())])
        .collect()
}

fn random_name(rng: &mut Rng) -> String {
    const NAME: &[u8] = b"abcxyz0189_";
    (0..=rng.below(5))
        .map(|_| char::from(NAME[rng.below(NAME.len())]))
        .collect()
}

fn random_node(rng: &mut Rng, depth: usize) -> Node {
    let kinds = if depth >= 4 { 3 } else { 5 };
    match rng.below(kinds) {
        0 => Node::Null,
        1 => Node::Bool(rng.below(2) == 0),
        2 => Node::String(random_string(rng)),
        3 => Node::Array(
            (0..rng.below(4))
                .map(|_| random_node(rng, depth + 1))
                .collect(),
        ),
        _ => {
            let mut members = BTreeMap::new();
            for _ in 0..rng.below(4) {
                members.insert(random_name(rng), random_node(rng, depth + 1));
            }
            Node::Object(members)
        }
    }
}

fn corpus(count: usize) -> Vec<Node> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    (0..count).map(|_| random_node(&mut rng, 0)).collect()
}

// ---------------------------------------------------------------------------
// 1. The crate against the oracle
// ---------------------------------------------------------------------------

#[test]
fn the_pinned_crate_matches_the_oracle_on_a_generated_corpus() {
    for node in corpus(3_000) {
        let bytes = node_to_canonical_vec(&node).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), oracle(&node), "{node:?}");
    }
}

#[test]
fn the_pinned_crate_matches_the_rfc_8785_serialization_example() {
    // RFC 8785 section 3.2.2 (the strings and literals; the numbers are outside this
    // profile and checked only against the crate below).
    let input = r#"{
        "numbers": [333333333.33333329, 1E30, 4.50, 2e-3, 0.000000000000000000000000001],
        "string": "\u20ac$\u000F\u000aA'\u0042\u0022\u005c\\\"/",
        "literals": [null, true, false]
    }"#;
    let expected =
        "{\"literals\":[null,true,false],\"numbers\":[333333333.3333333,1e+30,4.5,0.002,1e-27],\"string\":\"€$\\u000f\\nA'B\\\"\\\\\\\\\\\"/\"}";
    let value: serde_json::Value = serde_json::from_str(input).unwrap();
    let crate_output =
        String::from_utf8(serde_json_canonicalizer::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        crate_output, expected,
        "the pinned crate against the RFC text"
    );

    // The profile's part of the same example, through this crate's API and the oracle.
    let profile = obj(&[
        ("string", s("\u{20ac}$\u{f}\nA'B\"\\\\\"/")),
        (
            "literals",
            Node::Array(vec![Node::Null, Node::Bool(true), Node::Bool(false)]),
        ),
    ]);
    let ours = String::from_utf8(node_to_canonical_vec(&profile).unwrap()).unwrap();
    assert_eq!(ours, oracle(&profile));
    assert_eq!(
        ours,
        "{\"literals\":[null,true,false],\"string\":\"€$\\u000f\\nA'B\\\"\\\\\\\\\\\"/\"}"
    );
}

#[test]
fn the_pinned_crate_sorts_members_by_utf16_code_units_like_the_oracle() {
    // RFC 8785 section 3.2.3: a euro sign, a carriage return, a Hebrew letter, "1", a
    // surrogate-pair emoji, U+0080 and o-diaeresis. UTF-16 order differs from UTF-8 byte
    // order here (the emoji's first code unit 0xD83D sorts before U+FB33 but its UTF-8
    // sorts after), which is why the oracle sorts by `encode_utf16`.
    let names = [
        "\u{20ac}",
        "\r",
        "\u{fb33}",
        "1",
        "\u{1f600}",
        "\u{80}",
        "\u{f6}",
    ];
    let node = Node::Object(names.iter().map(|n| ((*n).to_owned(), s("v"))).collect());
    // Non-ASCII names are not legal in this profile, so the check goes to the crate
    // directly rather than through `node_to_canonical_vec`.
    let crate_output = String::from_utf8(serde_json_canonicalizer::to_vec(&node).unwrap()).unwrap();
    assert_eq!(crate_output, oracle(&node));
    let order: Vec<&str> = [
        "\r",
        "1",
        "\u{80}",
        "\u{f6}",
        "\u{20ac}",
        "\u{1f600}",
        "\u{fb33}",
    ]
    .to_vec();
    let expected = format!(
        "{{{}}}",
        order
            .iter()
            .map(|n| format!("{}:\"v\"", oracle_string(n)))
            .collect::<Vec<_>>()
            .join(",")
    );
    assert_eq!(crate_output, expected);
    // ... and the profile itself refuses those names.
    assert!(matches!(
        node_to_canonical_vec(&node),
        Err(CanonicalJsonError::BadMemberName)
    ));
}

#[test]
fn the_specs_own_export_manifest_vector_is_canonical_under_this_codec() {
    // vault-format.md lists `export_manifest_json` as hex. It must parse, re-encode to the
    // same bytes, and match the oracle, tying the document to the code.
    const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");
    let marker = "<!-- sync-vector: export_manifest_json=";
    let hex = SPEC
        .split(marker)
        .nth(1)
        .unwrap()
        .split(" -->")
        .next()
        .unwrap();
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let node = parse_node_lenient(&bytes).unwrap();
    assert_eq!(node_to_canonical_vec(&node).unwrap(), bytes);
    assert_eq!(oracle(&node).as_bytes(), bytes.as_slice());
}

#[test]
fn the_crate_alone_would_corrupt_large_integers_which_is_why_the_strictness_is_ours() {
    // The pinned crate formats a JSON number the way JavaScript would: a 64-bit integer
    // above 2^53 loses its low digits. This codec never hands it a number.
    let lossy = serde_json::json!({ "x": i64::MAX });
    assert_eq!(
        String::from_utf8(serde_json_canonicalizer::to_vec(&lossy).unwrap()).unwrap(),
        "{\"x\":9223372036854776000}"
    );
    #[derive(Serialize)]
    struct Holder {
        x: i64,
        y: u64,
    }
    let ours = to_canonical_vec(&Holder {
        x: i64::MAX,
        y: u64::MAX,
    })
    .unwrap();
    assert_eq!(
        String::from_utf8(ours).unwrap(),
        "{\"x\":\"9223372036854775807\",\"y\":\"18446744073709551615\"}"
    );
}

#[test]
fn only_the_to_vec_entry_point_of_the_crate_is_used() {
    // The crate's string helpers go through `from_utf8_unchecked`; policy is `to_vec` only.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in walk(&dir) {
        let source = std::fs::read_to_string(&entry).unwrap();
        for line in source.lines().filter(|l| !l.trim_start().starts_with("//")) {
            assert!(
                !line.contains("serde_json_canonicalizer::to_string")
                    && !line.contains("from_utf8_unchecked"),
                "{}: {line}",
                entry.display()
            );
        }
    }
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 2. The strictness this crate adds
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Ints {
    a: i8,
    b: i16,
    c: i32,
    d: i64,
    e: u8,
    f: u16,
    g: u32,
    h: u64,
    i: i128,
    j: u128,
}

#[test]
fn every_integer_width_is_a_decimal_string_and_round_trips_at_its_extremes() {
    let max = Ints {
        a: i8::MAX,
        b: i16::MAX,
        c: i32::MAX,
        d: i64::MAX,
        e: u8::MAX,
        f: u16::MAX,
        g: u32::MAX,
        h: u64::MAX,
        i: i128::MAX,
        j: u128::MAX,
    };
    let min = Ints {
        a: i8::MIN,
        b: i16::MIN,
        c: i32::MIN,
        d: i64::MIN,
        e: 0,
        f: 0,
        g: 0,
        h: 0,
        i: i128::MIN,
        j: 0,
    };
    for value in [max, min] {
        let bytes = to_canonical_vec(&value).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        // every value in the document is a string: no digit follows a colon
        assert!(
            !text.contains(":1") && !text.contains(":-") && !text.contains(":0"),
            "{text}"
        );
        assert_eq!(from_canonical_slice::<Ints>(&bytes).unwrap(), value);
    }
    let text = String::from_utf8(
        to_canonical_vec(&Ints {
            a: -1,
            b: 2,
            c: -3,
            d: 4,
            e: 5,
            f: 6,
            g: 7,
            h: 8,
            i: -9,
            j: 10,
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        text,
        "{\"a\":\"-1\",\"b\":\"2\",\"c\":\"-3\",\"d\":\"4\",\"e\":\"5\",\"f\":\"6\",\"g\":\"7\",\"h\":\"8\",\"i\":\"-9\",\"j\":\"10\"}"
    );
}

fn ints_with(field: &str, value: &str) -> String {
    let base = [
        ("a", "0"),
        ("b", "0"),
        ("c", "0"),
        ("d", "0"),
        ("e", "0"),
        ("f", "0"),
        ("g", "0"),
        ("h", "0"),
        ("i", "0"),
        ("j", "0"),
    ];
    let body: Vec<String> = base
        .iter()
        .map(|(k, v)| format!("\"{k}\":\"{}\"", if *k == field { value } else { v }))
        .collect();
    format!("{{{}}}", body.join(","))
}

#[test]
fn integer_strings_must_match_the_grammar_and_fit_their_field() {
    let decode =
        |field: &str, value: &str| from_canonical_slice::<Ints>(ints_with(field, value).as_bytes());
    // Out of range for the field.
    for (field, value) in [
        ("a", "128"),
        ("a", "-129"),
        ("e", "256"),
        ("e", "-1"),
        ("d", "9223372036854775808"),
        ("d", "-9223372036854775809"),
        ("h", "18446744073709551616"),
        ("h", "-1"),
        ("j", "340282366920938463463374607431768211456"),
    ] {
        assert!(decode(field, value).is_err(), "{field}={value}");
    }
    // Bad syntax: sign, leading zero, fraction, exponent, space, empty, hex, a lone minus.
    for value in [
        "+1", "01", "00", "-0", "-00", "1.0", "1e3", " 1", "1 ", "", "0x10", "-", "--1", "1_000",
        "١",
    ] {
        assert_eq!(
            decode("d", value).unwrap_err(),
            CanonicalJsonError::BadInteger,
            "{value:?}"
        );
    }
    // Zero has exactly one spelling.
    assert!(decode("d", "0").is_ok());
    // A number or another type where an integer string is expected.
    let number = ints_with("d", "0").replace("\"d\":\"0\"", "\"d\":0");
    assert_eq!(
        from_canonical_slice::<Ints>(number.as_bytes()).unwrap_err(),
        CanonicalJsonError::NumberNotAllowed
    );
    let boolean = ints_with("d", "0").replace("\"d\":\"0\"", "\"d\":true");
    assert!(matches!(
        from_canonical_slice::<Ints>(boolean.as_bytes()),
        Err(CanonicalJsonError::Decode(_))
    ));
}

#[test]
fn a_json_number_anywhere_is_refused() {
    for doc in [
        "0",
        "1",
        "-1",
        "1.5",
        "1e2",
        "-0",
        "[1]",
        "[0]",
        "{\"a\":1}",
        "{\"a\":[1]}",
        "{\"a\":1.0}",
        "{\"a\":-0}",
        "[true,2]",
    ] {
        assert_eq!(
            parse_node_lenient(doc.as_bytes()).unwrap_err(),
            CanonicalJsonError::NumberNotAllowed,
            "{doc}"
        );
    }
}

#[test]
fn floating_point_values_cannot_be_serialized() {
    #[derive(Serialize)]
    struct F32(f32);
    #[derive(Serialize)]
    struct F64(f64);
    assert_eq!(
        to_canonical_vec(&F32(1.5)).unwrap_err(),
        CanonicalJsonError::Float
    );
    assert_eq!(
        to_canonical_vec(&F64(f64::NAN)).unwrap_err(),
        CanonicalJsonError::Float
    );
    assert_eq!(
        to_canonical_vec(&F64(f64::INFINITY)).unwrap_err(),
        CanonicalJsonError::Float
    );
    assert_eq!(
        to_canonical_vec(&F64(0.0)).unwrap_err(),
        CanonicalJsonError::Float
    );
    #[derive(Debug, Deserialize, Serialize)]
    struct Wants(f64);
    assert_eq!(
        from_canonical_slice::<Wants>(b"\"1\"").unwrap_err(),
        CanonicalJsonError::Float
    );
}

#[test]
fn duplicate_member_names_are_refused_however_they_are_spelled() {
    for doc in [
        r#"{"a":"1","a":"2"}"#,
        r#"{"a":"1","b":"2","a":"3"}"#,
        r#"{"a":"1","\u0061":"2"}"#,
        r#"{"x":{"a":"1","a":"1"}}"#,
        r#"[{"a":null,"a":null}]"#,
    ] {
        assert!(
            matches!(
                parse_node_lenient(doc.as_bytes()),
                Err(CanonicalJsonError::DuplicateMember)
            ),
            "{doc}"
        );
    }
    // The same name in different objects is fine.
    assert!(parse_node_lenient(br#"{"a":{"a":"1"},"b":{"a":"2"}}"#).is_ok());
}

#[test]
fn member_names_are_ascii_lowercase_digits_and_underscore() {
    for name in [
        "A", "a-b", "a b", "a.b", "\u{e9}", "\u{20ac}", "", "Zed", "a/b",
    ] {
        let doc = format!("{{{}:\"1\"}}", oracle_string(name));
        assert!(
            matches!(
                parse_node_lenient(doc.as_bytes()),
                Err(CanonicalJsonError::BadMemberName)
            ),
            "{name:?}"
        );
    }
    // A NUL in a name is refused even earlier, as a malformed string.
    assert!(matches!(
        parse_node_lenient(br#"{"a\u0000":"1"}"#),
        Err(CanonicalJsonError::Malformed { .. })
    ));
    for name in ["a", "_", "0", "a_0", "snake_case_9"] {
        let doc = format!("{{\"{name}\":\"1\"}}");
        assert!(parse_node_lenient(doc.as_bytes()).is_ok(), "{name}");
    }
}

#[test]
fn lone_surrogate_escapes_are_refused() {
    for doc in [
        r#""\ud800""#,
        r#""\udbff""#,
        r#""\udc00""#,
        r#""\udfff""#,
        r#""\ud800\u0041""#,
        r#""\ud800x""#,
        r#""\ud800\ud800""#,
        r#""\udc00\ud800""#,
        r#""a\ud83d""#,
        r#"{"a":"\ude00"}"#,
    ] {
        assert!(
            matches!(
                parse_node_lenient(doc.as_bytes()),
                Err(CanonicalJsonError::Malformed {
                    reason: "lone surrogate escape",
                    ..
                })
            ),
            "{doc}"
        );
    }
    // A valid pair parses (but its escaped spelling is not canonical: see below).
    assert_eq!(
        parse_node_lenient(br#""\ud83d\ude00""#).unwrap(),
        s("\u{1f600}")
    );
    assert!(parse_node_lenient(br#""\u0001\u001f\u007f\uffff""#).is_ok());
}

#[test]
fn invalid_utf8_is_refused() {
    for bytes in [
        &b"\"\xff\""[..],
        b"\"\xc0\x80\"",         // overlong NUL
        b"\"\xed\xa0\x80\"",     // an encoded surrogate
        b"\"\xf4\x90\x80\x80\"", // above U+10FFFF
        b"\"\xc3\"",             // truncated sequence
        b"\"\xe2\x82\"",
        b"\xff",
        b"{\"a\":\"\x80\"}",
    ] {
        assert!(
            matches!(
                parse_node_lenient(bytes),
                Err(CanonicalJsonError::Malformed {
                    reason: "invalid UTF-8",
                    ..
                })
            ),
            "{bytes:?}"
        );
    }
}

#[test]
fn malformed_documents_are_refused_with_a_reason_and_never_panic() {
    for doc in [
        "",
        " ",
        "{",
        "}",
        "[",
        "]",
        "{\"a\"}",
        "{\"a\":}",
        "{\"a\":\"1\",}",
        "[\"1\",]",
        "[,]",
        "{,}",
        "{\"a\":\"1\" \"b\":\"2\"}",
        "tru",
        "nul",
        "True",
        "NULL",
        "\"abc",
        "\"a\nb\"",
        "\"a\tb\"",
        "\"\\x41\"",
        "\"\\u12\"",
        "\"\\uzzzz\"",
        "{\"a\":\"1\"} x",
        "{}{}",
        "[] []",
        "'a'",
        "{'a':'1'}",
        "{a:\"1\"}",
        "\"\\\"",
        "\u{feff}{}",
    ] {
        assert!(parse_node_lenient(doc.as_bytes()).is_err(), "{doc:?}");
    }
    for doc in [
        r#"{}"#,
        r#"[]"#,
        r#"null"#,
        r#"true"#,
        r#"false"#,
        r#""""#,
        r#"[null,true,"x",{},[]]"#,
    ] {
        assert!(parse_node_lenient(doc.as_bytes()).is_ok(), "{doc}");
    }
}

#[test]
fn nesting_depth_is_bounded_on_parsing_and_on_encoding() {
    let nested = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
    // The outermost container is level 1; MAX_DEPTH levels are allowed, one more is not.
    assert!(parse_node_lenient(nested(MAX_DEPTH).as_bytes()).is_ok());
    assert_eq!(
        parse_node_lenient(nested(MAX_DEPTH + 1).as_bytes()).unwrap_err(),
        CanonicalJsonError::TooDeep(MAX_DEPTH)
    );
    assert!(parse_node_lenient(br#"{"a":{"a":"x"}}"#).is_ok());
    // A pathological depth is a clean error, not a stack overflow.
    assert!(parse_node_lenient(nested(1_000_000).as_bytes()).is_err());

    // Encoding enforces the same limit, so no origin can emit a document nobody can decode.
    let tree = |levels: usize| {
        let mut node = Node::Array(Vec::new());
        for _ in 1..levels {
            node = Node::Array(vec![node]);
        }
        node
    };
    assert!(node_to_canonical_vec(&tree(MAX_DEPTH)).is_ok());
    assert_eq!(
        node_to_canonical_vec(&tree(MAX_DEPTH + 1)).unwrap_err(),
        CanonicalJsonError::TooDeep(MAX_DEPTH)
    );
    let bytes = node_to_canonical_vec(&tree(MAX_DEPTH)).unwrap();
    assert_eq!(parse_canonical_node(&bytes).unwrap(), tree(MAX_DEPTH));
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Pair {
    first: String,
    second: Option<String>,
}

#[test]
fn a_valid_but_non_canonical_document_is_refused() {
    let canonical = br#"{"first":"x","second":null}"#;
    assert_eq!(
        from_canonical_slice::<Pair>(canonical).unwrap(),
        Pair {
            first: "x".into(),
            second: None
        }
    );
    for doc in [
        r#"{"second":null,"first":"x"}"#,             // member order
        r#"{ "first":"x","second":null}"#,            // whitespace
        "{\"first\":\"x\",\"second\":null}\n",        // trailing newline
        r#"{"first":"\u0078","second":null}"#,        // an escaped ordinary character
        r#"{"first":"x","second":null,"extra":"1"}"#, // an unknown member (ignored by serde)
        r#"{"first":"\/","second":null}"#,            // an escaped solidus
        r#"{"first":"\u00e9","second":null}"#,        // an escaped non-ASCII character
        r#"{"first":"\ud83d\ude00","second":null}"#,  // an escaped surrogate pair
        r#"{"first":"\u000A","second":null}"#,        // the wrong spelling of a control escape
    ] {
        assert_eq!(
            from_canonical_slice::<Pair>(doc.as_bytes()).unwrap_err(),
            CanonicalJsonError::NotCanonical,
            "{doc}"
        );
    }
    // A missing optional member is also not the canonical spelling of `None`.
    assert_eq!(
        from_canonical_slice::<Pair>(br#"{"first":"x"}"#).unwrap_err(),
        CanonicalJsonError::NotCanonical
    );
}

// ---------------------------------------------------------------------------
// Typed round trips
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Shape {
    Unit,
    Newtype(String),
    Tuple(i64, bool),
    Struct {
        id_value: u32,
        label: Option<String>,
    },
    Nested(Box<Shape>),
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Everything {
    flag: bool,
    text: String,
    ch: char,
    maybe: Option<i64>,
    list: Vec<Shape>,
    pair: (u8, String),
    by_number: BTreeMap<u32, String>,
    nothing: (),
}

#[test]
fn typed_values_round_trip_with_the_exact_expected_bytes() {
    let value = Everything {
        flag: true,
        text: "caf\u{e9} \u{1f600} \"q\" \\ \u{1}".to_owned(),
        ch: 'x',
        maybe: Some(-5),
        list: vec![
            Shape::Unit,
            Shape::Newtype("n".into()),
            Shape::Tuple(i64::MIN, false),
            Shape::Struct {
                id_value: 7,
                label: None,
            },
            Shape::Nested(Box::new(Shape::Struct {
                id_value: u32::MAX,
                label: Some("L".into()),
            })),
        ],
        pair: (255, "p".into()),
        by_number: BTreeMap::from([(10, "ten".to_owned()), (9, "nine".to_owned())]),
        nothing: (),
    };
    let bytes = to_canonical_vec(&value).unwrap();
    let expected = concat!(
        "{\"by_number\":{\"10\":\"ten\",\"9\":\"nine\"},\"ch\":\"x\",\"flag\":true,",
        "\"list\":[\"unit\",{\"newtype\":\"n\"},{\"tuple\":[\"-9223372036854775808\",false]},",
        "{\"struct\":{\"id_value\":\"7\",\"label\":null}},",
        "{\"nested\":{\"struct\":{\"id_value\":\"4294967295\",\"label\":\"L\"}}}],",
        "\"maybe\":\"-5\",\"nothing\":null,\"pair\":[\"255\",\"p\"],",
        "\"text\":\"café \u{1f600} \\\"q\\\" \\\\ \\u0001\"}"
    );
    assert_eq!(String::from_utf8(bytes.clone()).unwrap(), expected);
    assert_eq!(from_canonical_slice::<Everything>(&bytes).unwrap(), value);
    // "10" sorts before "9": member names sort as strings, not as the numbers they spell.
    assert!(expected.find("\"10\"").unwrap() < expected.find("\"9\"").unwrap());
}

#[test]
fn an_unknown_variant_or_a_wrong_shape_is_a_decode_error_that_leaks_no_value() {
    let secret = "SECRET-4242.50";
    let doc = format!("{{\"no_such_variant\":\"{secret}\"}}");
    let err = from_canonical_slice::<Shape>(doc.as_bytes()).unwrap_err();
    assert!(matches!(err, CanonicalJsonError::Decode(_)));
    assert!(!err.to_string().contains(secret), "{err}");
    assert!(!format!("{err:?}").contains(secret), "{err:?}");
    // A multi-member object is not a variant.
    assert!(from_canonical_slice::<Shape>(br#"{"unit":null,"newtype":"x"}"#).is_err());
}

#[test]
fn raw_bytes_and_non_string_map_keys_are_refused() {
    struct Bytes;
    impl Serialize for Bytes {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_bytes(&[1, 2, 3])
        }
    }
    assert!(matches!(
        to_canonical_vec(&Bytes),
        Err(CanonicalJsonError::Unsupported(_))
    ));
    let mut by_bool = BTreeMap::new();
    by_bool.insert(true, "x");
    assert!(matches!(
        to_canonical_vec(&by_bool),
        Err(CanonicalJsonError::Unsupported(_))
    ));
    // A free-form string key outside [a-z0-9_] is refused.
    let mut free = BTreeMap::new();
    free.insert("Not A Name".to_owned(), "x");
    assert!(matches!(
        to_canonical_vec(&free),
        Err(CanonicalJsonError::BadMemberName)
    ));
}

// ---------------------------------------------------------------------------
// 3. The parser against serde_json
// ---------------------------------------------------------------------------

/// Escape every non-ASCII character as `\uXXXX` (a surrogate pair for astral scalars).
fn ascii_escaped(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut units = [0_u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{:04X}", unit));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn ascii_json(node: &Node) -> String {
    match node {
        Node::Null => "null".to_owned(),
        Node::Bool(b) => b.to_string(),
        Node::String(s) => ascii_escaped(s),
        Node::Array(items) => format!(
            "[ {} ]",
            items.iter().map(ascii_json).collect::<Vec<_>>().join(" , ")
        ),
        Node::Object(members) => format!(
            "{{ {} }}",
            members
                .iter()
                .map(|(k, v)| format!("{} : {}", ascii_escaped(k), ascii_json(v)))
                .collect::<Vec<_>>()
                .join(" , ")
        ),
    }
}

#[test]
fn the_parser_agrees_with_serde_json_on_valid_documents_in_several_spellings() {
    for node in corpus(1_500) {
        let canonical = oracle(&node);
        let escaped = ascii_json(&node);
        let pretty = serde_json::to_string_pretty(&serde_json::to_value(&node).unwrap()).unwrap();
        for spelling in [&canonical, &escaped, &pretty] {
            assert_eq!(
                parse_node_lenient(spelling.as_bytes()).unwrap(),
                node,
                "{spelling}"
            );
            let reference: serde_json::Value = serde_json::from_str(spelling).unwrap();
            assert_eq!(reference, serde_json::to_value(&node).unwrap());
        }
    }
}

#[test]
fn whatever_the_parser_accepts_serde_json_accepts_too_and_it_never_panics() {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    for node in corpus(600) {
        let original = oracle(&node).into_bytes();
        for _ in 0..12 {
            let mut bytes = original.clone();
            match rng.below(4) {
                0 if !bytes.is_empty() => {
                    let at = rng.below(bytes.len());
                    bytes[at] = u8::try_from(rng.below(256)).unwrap();
                }
                1 if !bytes.is_empty() => bytes.truncate(rng.below(bytes.len())),
                2 => bytes.insert(rng.below(bytes.len() + 1), b"{}[]\",:\\ 1-"[rng.below(11)]),
                _ if !bytes.is_empty() => {
                    bytes.remove(rng.below(bytes.len()));
                }
                _ => {}
            }
            if parse_node_lenient(&bytes).is_ok() {
                // Our grammar is a strict subset of JSON.
                assert!(
                    serde_json::from_slice::<serde_json::Value>(&bytes).is_ok(),
                    "{bytes:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Rejection vectors, read from vault-format.md C.1.2
// ---------------------------------------------------------------------------

fn spec_reject_vectors() -> Vec<(String, Vec<u8>)> {
    const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");
    SPEC.split("<!-- cj-reject: ")
        .skip(1)
        .map(|rest| {
            let entry = rest.split(" -->").next().unwrap();
            let (name, hex) = entry.split_once('=').unwrap();
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            (name.to_owned(), bytes)
        })
        .collect()
}

#[test]
fn every_rejection_vector_of_the_spec_is_refused_by_the_canonical_parser() {
    let vectors = spec_reject_vectors();
    assert_eq!(vectors.len(), 25, "the spec lists 25 rejection vectors");
    for (name, bytes) in &vectors {
        assert!(
            parse_canonical_node(bytes).is_err(),
            "{name} must be refused: {:?}",
            String::from_utf8_lossy(bytes)
        );
    }
    // The table the document shows to people lists the same names as the markers.
    const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");
    for (name, _) in &vectors {
        assert!(
            SPEC.contains(&format!("| `{name}` |")),
            "{name} is missing from the table"
        );
    }
}

#[test]
fn the_rejection_vectors_fail_for_the_reasons_the_spec_gives() {
    use CanonicalJsonError as E;
    let reason = |name: &str| {
        let (_, bytes) = spec_reject_vectors()
            .into_iter()
            .find(|(n, _)| n == name)
            .unwrap();
        parse_canonical_node(&bytes).unwrap_err()
    };
    // Spelling only: the content is fine, the spelling is not canonical.
    for name in [
        "whitespace_between_members",
        "trailing_newline",
        "members_out_of_order",
        "escaped_ordinary_character",
        "escaped_solidus",
        "uppercase_hex_escape",
        "escaped_non_ascii_character",
        "escaped_control_with_wrong_spelling",
        "escaped_surrogate_pair",
    ] {
        assert_eq!(reason(name), E::NotCanonical, "{name}");
    }
    assert!(matches!(reason("empty_member_name"), E::BadMemberName));
    assert_eq!(reason("nested_129_levels"), E::TooDeep(MAX_DEPTH));
    assert_eq!(reason("a_json_number"), E::NumberNotAllowed);
    assert_eq!(reason("a_float"), E::NumberNotAllowed);
    assert!(matches!(reason("duplicate_member"), E::DuplicateMember));
    assert!(matches!(
        reason("duplicate_member_escaped"),
        E::DuplicateMember
    ));
    assert!(matches!(reason("member_name_uppercase"), E::BadMemberName));
    assert!(matches!(reason("member_name_with_dot"), E::BadMemberName));
    for name in [
        "lone_high_surrogate",
        "lone_low_surrogate",
        "nul_escape",
        "raw_control_character",
        "leading_byte_order_mark",
        "trailing_comma",
        "invalid_utf8_in_a_string",
        "overlong_utf8_in_a_string",
    ] {
        assert!(matches!(reason(name), E::Malformed { .. }), "{name}");
    }
}

// ---------------------------------------------------------------------------
// One encoding per value
// ---------------------------------------------------------------------------

#[test]
fn a_some_whose_content_is_null_is_refused_so_none_and_some_none_never_share_bytes() {
    #[derive(Debug, Serialize)]
    struct Patch {
        field: Option<Option<String>>,
    }
    #[derive(Debug, Serialize)]
    struct UnitInSome {
        field: Option<()>,
    }
    #[derive(Debug, Serialize)]
    struct Marker;
    #[derive(Debug, Serialize)]
    struct MarkerInSome {
        field: Option<Marker>,
    }
    // None is fine, and so is a real value ...
    assert_eq!(
        to_canonical_vec(&Patch { field: None }).unwrap(),
        br#"{"field":null}"#
    );
    assert_eq!(
        to_canonical_vec(&Patch {
            field: Some(Some("x".into()))
        })
        .unwrap(),
        br#"{"field":"x"}"#
    );
    // ... but "present and cleared" would encode exactly like "absent".
    for refused in [
        to_canonical_vec(&Patch { field: Some(None) }),
        to_canonical_vec(&UnitInSome { field: Some(()) }),
        to_canonical_vec(&MarkerInSome {
            field: Some(Marker),
        }),
    ] {
        assert!(
            matches!(refused, Err(CanonicalJsonError::Unsupported(_))),
            "{refused:?}"
        );
    }
    // An empty string is a real value, not null.
    assert!(to_canonical_vec(&Patch {
        field: Some(Some(String::new()))
    })
    .is_ok());
}

#[test]
fn u0000_is_refused_in_a_string_on_every_path() {
    assert!(to_canonical_vec(&"a\u{0}b").is_err());
    assert!(to_canonical_vec(&'\u{0}').is_err());
    assert!(node_to_canonical_vec(&Node::String("\u{0}".into())).is_err());
    assert!(parse_node_lenient(br#""\u0000""#).is_err());
    assert!(parse_node_lenient(b"\"\x00\"").is_err());
    // Every other control character is fine (escaped on the way out).
    for c in 1_u8..=0x1f {
        let text = char::from(c).to_string();
        assert!(to_canonical_vec(&text).is_ok(), "{c:#x}");
    }
}

#[test]
fn escape_heavy_strings_agree_with_the_oracle_and_survive_a_canonical_round_trip() {
    let mut texts: Vec<String> = (1_u8..=0x1f).map(|c| char::from(c).to_string()).collect();
    texts.extend(
        [
            "\"",
            "\\",
            "/",
            "\u{7f}",
            "\u{80}",
            "\u{2028}",
            "\u{2029}",
            "\u{ffff}",
            "\u{10000}",
            "\u{10ffff}",
            "\u{1f600}\u{1f600}",
            "\\u0041",
            "a\\\"b\\\\c",
            "\u{301}\u{e9}",
            "\r\n\t\u{8}\u{c}",
        ]
        .map(str::to_owned),
    );
    texts.push((1_u8..=0x1f).map(char::from).collect());
    for text in texts {
        let node = obj(&[("v", Node::String(text.clone()))]);
        let bytes = node_to_canonical_vec(&node).unwrap();
        assert_eq!(
            String::from_utf8(bytes.clone()).unwrap(),
            oracle(&node),
            "{text:?}"
        );
        assert_eq!(parse_canonical_node(&bytes).unwrap(), node, "{text:?}");
    }
}

#[test]
fn errors_never_print_the_documents_member_names_or_values() {
    for doc in [
        r#"{"Secret-Name-1250":"5"}"#,
        r#"{"secret_name_1250":"5","secret_name_1250":"6"}"#,
        r#"{"secret_name_1250":"\ud800"}"#,
        r#"{"secret_name_1250":1250}"#,
    ] {
        let err = parse_canonical_node(doc.as_bytes()).unwrap_err();
        for text in [err.to_string(), format!("{err:?}")] {
            assert!(!text.to_lowercase().contains("secret"), "{doc}: {text}");
            assert!(!text.contains("1250"), "{doc}: {text}");
        }
    }
    let mut map = BTreeMap::new();
    map.insert("Secret-Name-1250".to_owned(), "x");
    let err = to_canonical_vec(&map).unwrap_err();
    assert!(
        !err.to_string().to_lowercase().contains("secret")
            && !format!("{err:?}").to_lowercase().contains("secret")
    );
}

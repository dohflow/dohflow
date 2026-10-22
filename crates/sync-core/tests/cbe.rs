//! CBE1 (vault-format.md C.1.1): each kind encodes to its one byte string and parses back, and
//! the reader rejects every malformed input the spec lists.

use sync_core::cbe::{Reader, WireError, Writer};

#[test]
fn every_kind_has_the_documented_bytes_and_parses_back() {
    let mut w = Writer::with_label("test/kinds");
    w.u8(0xab)
        .u16(0x0102)
        .u32(0x0304_0506)
        .u64(0x0708_090a_0b0c_0d0e)
        .i64(-2)
        .fixed(&[0x11; 4]);
    w.bytes(8, b"hi").unwrap();
    w.text(8, "h\u{e9}").unwrap();
    w.opt(None::<&[u8; 2]>, |w, v| {
        w.fixed(v);
    });
    w.opt(Some(&[0x22, 0x33]), |w, v| {
        w.fixed(v);
    });
    let bytes = w.finish();

    let expected: Vec<u8> = [
        &[0x00, 0x0a][..],
        b"test/kinds",
        &[0xab],
        &[0x01, 0x02],
        &[0x03, 0x04, 0x05, 0x06],
        &[0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e],
        &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe],
        &[0x11; 4],
        &[0, 0, 0, 2],
        b"hi",
        &[0, 0, 0, 3],
        "h\u{e9}".as_bytes(),
        &[0],
        &[1, 0x22, 0x33],
    ]
    .concat();
    assert_eq!(bytes, expected);

    let mut r = Reader::new(&bytes);
    r.label("test/kinds").unwrap();
    assert_eq!(r.u8().unwrap(), 0xab);
    assert_eq!(r.u16().unwrap(), 0x0102);
    assert_eq!(r.u32().unwrap(), 0x0304_0506);
    assert_eq!(r.u64().unwrap(), 0x0708_090a_0b0c_0d0e);
    assert_eq!(r.i64().unwrap(), -2);
    assert_eq!(r.fixed::<4>().unwrap(), [0x11; 4]);
    assert_eq!(r.bytes(8).unwrap(), b"hi");
    assert_eq!(r.text(8).unwrap(), "h\u{e9}");
    assert_eq!(r.opt(|r| r.fixed::<2>()).unwrap(), None);
    assert_eq!(r.opt(|r| r.fixed::<2>()).unwrap(), Some([0x22, 0x33]));
    r.finish().unwrap();
}

#[test]
fn trailing_bytes_truncation_and_bad_presence_are_refused() {
    let mut w = Writer::with_label("t/x");
    w.u32(7);
    let mut bytes = w.finish();
    bytes.push(9);
    let mut r = Reader::new(&bytes);
    r.label("t/x").unwrap();
    assert_eq!(r.u32().unwrap(), 7);
    assert_eq!(r.finish(), Err(WireError::TrailingBytes));

    let mut r = Reader::new(&bytes[..bytes.len() - 3]);
    r.label("t/x").unwrap();
    assert_eq!(r.u32(), Err(WireError::Truncated));

    for presence in [2_u8, 0x80, 0xff] {
        let mut r = Reader::new(std::slice::from_ref(&presence));
        assert_eq!(r.opt(|r| r.u8()), Err(WireError::BadPresence));
    }
}

#[test]
fn lengths_are_checked_against_their_maximum_and_the_bytes_present() {
    let mut w = Writer::with_label("t/x");
    assert_eq!(w.bytes(2, b"abc").err(), Some(WireError::TooLong));
    assert_eq!(w.text(2, "abc").err(), Some(WireError::TooLong));

    // A stated length above the maximum.
    let mut r = Reader::new(&[0, 0, 0, 3, b'a', b'b', b'c']);
    assert_eq!(r.bytes(2), Err(WireError::TooLong));
    // A length that claims more than is there is refused before anything is taken.
    let mut r = Reader::new(&[0xff, 0xff, 0xff, 0xff, 1, 2, 3]);
    assert_eq!(r.bytes(usize::MAX), Err(WireError::LengthExceedsInput));
    // Invalid UTF-8 in a text.
    let mut r = Reader::new(&[0, 0, 0, 2, 0xc3, 0x28]);
    assert_eq!(r.text(8), Err(WireError::InvalidUtf8));
    // The maximum counts bytes, not characters.
    let mut w = Writer::with_label("t/x");
    assert!(w.text(2, "\u{e9}").is_ok());
    assert_eq!(w.text(1, "\u{e9}").err(), Some(WireError::TooLong));
}

#[test]
fn labels_are_checked_for_shape_and_identity() {
    let make = |label: &str| {
        let mut bytes = (label.len() as u16).to_be_bytes().to_vec();
        bytes.extend_from_slice(label.as_bytes());
        bytes
    };
    assert_eq!(
        Reader::new(&make("ok/label-1_x.y")).label("ok/label-1_x.y"),
        Ok(())
    );
    assert_eq!(
        Reader::new(&make("ok/other")).label("ok/label"),
        Err(WireError::LabelMismatch)
    );
    assert_eq!(
        Reader::new(&make("Upper")).label("upper"),
        Err(WireError::BadLabel)
    );
    assert_eq!(
        Reader::new(&make("has space")).label("x"),
        Err(WireError::BadLabel)
    );
    assert_eq!(Reader::new(&make("")).label("x"), Err(WireError::BadLabel));
    assert_eq!(
        Reader::new(&make(&"a".repeat(65))).label("x"),
        Err(WireError::BadLabel)
    );
    assert_eq!(
        Reader::new(&make(&"a".repeat(64))).label(&"a".repeat(64)),
        Ok(())
    );
    // A truncated label.
    assert_eq!(
        Reader::new(&[0, 5, b'a']).label("aaaaa"),
        Err(WireError::Truncated)
    );
}

#[test]
#[should_panic(expected = "bad CBE1 label")]
fn building_a_structure_with_a_bad_label_is_a_programming_error() {
    let _ = Writer::with_label("Bad Label");
}

#[test]
fn only_a_wire_failure_other_than_unsupported_versions_is_a_malformed_object() {
    assert!(WireError::TrailingBytes.is_malformed());
    assert!(WireError::Truncated.is_malformed());
    assert!(!WireError::UnsupportedProtocol.is_malformed());
    assert!(!WireError::UnsupportedContainer.is_malformed());
}

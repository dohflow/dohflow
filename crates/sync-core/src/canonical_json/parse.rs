//! A strict JSON parser for the restricted model (C.1.2).
//!
//! It exists, rather than reusing `serde_json`'s parser, because the format needs exact
//! and *classified* rejection: any JSON number, a duplicate member name, a member name
//! outside ASCII `[a-z0-9_]`, a lone surrogate escape, a raw control character in a
//! string, invalid UTF-8, and nesting beyond a fixed depth. It accepts any JSON
//! whitespace and any escape spelling; the caller decides whether the *document* is
//! canonical by re-encoding the value (see [`super::from_canonical_slice`]).

use std::collections::BTreeMap;

use super::error::CanonicalJsonError;
use super::node::{insert_member, Node};

/// The deepest nesting accepted. It bounds the parser's recursion.
pub const MAX_DEPTH: usize = 128;

/// Parse a document into a [`Node`], enforcing every rule of the restricted model.
///
/// # Errors
/// [`CanonicalJsonError`] naming the rule that was broken.
pub fn parse_node(bytes: &[u8]) -> Result<Node, CanonicalJsonError> {
    let text = std::str::from_utf8(bytes).map_err(|e| CanonicalJsonError::Malformed {
        offset: e.valid_up_to(),
        reason: "invalid UTF-8",
    })?;
    let mut parser = Parser { text, pos: 0 };
    parser.skip_whitespace();
    let node = parser.value(0)?;
    parser.skip_whitespace();
    if parser.pos != text.len() {
        return Err(parser.malformed("characters after the document"));
    }
    Ok(node)
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes().get(self.pos).copied()
    }

    fn malformed(&self, reason: &'static str) -> CanonicalJsonError {
        CanonicalJsonError::Malformed {
            offset: self.pos,
            reason,
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8, reason: &'static str) -> Result<(), CanonicalJsonError> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.malformed(reason))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Node, CanonicalJsonError> {
        if depth > MAX_DEPTH {
            return Err(CanonicalJsonError::TooDeep(MAX_DEPTH));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Node::String),
            Some(b't') => self.literal("true", Node::Bool(true)),
            Some(b'f') => self.literal("false", Node::Bool(false)),
            Some(b'n') => self.literal("null", Node::Null),
            Some(b'-' | b'0'..=b'9') => Err(CanonicalJsonError::NumberNotAllowed),
            Some(_) => Err(self.malformed("unexpected character")),
            None => Err(self.malformed("unexpected end of input")),
        }
    }

    fn literal(&mut self, word: &'static str, node: Node) -> Result<Node, CanonicalJsonError> {
        if self.bytes()[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(node)
        } else {
            Err(self.malformed("invalid literal"))
        }
    }

    fn array(&mut self, depth: usize) -> Result<Node, CanonicalJsonError> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Node::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Node::Array(items));
                }
                _ => return Err(self.malformed("expected ',' or ']'")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Node, CanonicalJsonError> {
        self.pos += 1; // '{'
        let mut members = BTreeMap::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Node::Object(members));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.malformed("expected a member name"));
            }
            let name = self.string()?;
            self.skip_whitespace();
            self.expect(b':', "expected ':'")?;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            insert_member(&mut members, name, value)?;
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Node::Object(members));
                }
                _ => return Err(self.malformed("expected ',' or '}'")),
            }
        }
    }

    fn string(&mut self) -> Result<String, CanonicalJsonError> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            // Copy the run of ordinary characters up to the next quote, backslash or
            // control byte. The text is valid UTF-8 and these delimiters are ASCII, so the
            // slice always ends on a character boundary.
            let start = self.pos;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(&self.text[start..self.pos]);
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    out.push(self.escape()?);
                }
                Some(_) => return Err(self.malformed("control character in a string")),
                None => return Err(self.malformed("unterminated string")),
            }
        }
    }

    fn escape(&mut self) -> Result<char, CanonicalJsonError> {
        let Some(b) = self.peek() else {
            return Err(self.malformed("unterminated escape"));
        };
        self.pos += 1;
        Ok(match b {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => self.unicode_escape()?,
            _ => return Err(self.malformed("invalid escape")),
        })
    }

    fn hex4(&mut self) -> Result<u32, CanonicalJsonError> {
        let mut unit = 0_u32;
        for _ in 0..4 {
            let digit = match self.peek() {
                Some(b @ b'0'..=b'9') => b - b'0',
                Some(b @ b'a'..=b'f') => b - b'a' + 10,
                Some(b @ b'A'..=b'F') => b - b'A' + 10,
                _ => return Err(self.malformed("invalid \\u escape")),
            };
            unit = unit * 16 + u32::from(digit);
            self.pos += 1;
        }
        Ok(unit)
    }

    fn unicode_escape(&mut self) -> Result<char, CanonicalJsonError> {
        let unit = self.hex4()?;
        match unit {
            0xD800..=0xDBFF => {
                // A high surrogate must be followed by an escaped low surrogate.
                if self.bytes()[self.pos..].starts_with(b"\\u") {
                    self.pos += 2;
                    let low = self.hex4()?;
                    if (0xDC00..=0xDFFF).contains(&low) {
                        let code = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                        return char::from_u32(code)
                            .ok_or_else(|| self.malformed("invalid surrogate pair"));
                    }
                }
                Err(self.malformed("lone surrogate escape"))
            }
            0xDC00..=0xDFFF => Err(self.malformed("lone surrogate escape")),
            _ => char::from_u32(unit).ok_or_else(|| self.malformed("invalid \\u escape")),
        }
    }
}

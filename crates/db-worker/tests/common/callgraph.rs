//! The source scanner both call-graph ratchets share (moved out of `apply_clock_ratchet.rs`):
//! comments and literal contents blanked with their positions kept, function bodies by name, and
//! calls by name. Name-based and conservative: it can over-report (two functions sharing a name),
//! never under-report a call written in the usual form.

use std::fs;
use std::path::{Path, PathBuf};

pub fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read_dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Replace comments and the contents of string and character literals with spaces, keeping
/// every other byte (and every newline), so prose and text that mention a call are not
/// mistaken for one and braces inside strings do not confuse function-body matching.
pub fn blank_noise(source: &str) -> String {
    let b = source.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let blank = |c: u8| if c == b'\n' { b'\n' } else { b' ' };
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    out.extend_from_slice(b"  ");
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    out.extend_from_slice(b"  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(b[i]));
                    i += 1;
                }
            }
        } else if c == b'r' && (i == 0 || !is_ident(b[i - 1])) && raw_string_len(&b[i..]).is_some()
        {
            let len = raw_string_len(&b[i..]).unwrap();
            out.push(b'r');
            out.extend(b[i + 1..i + len].iter().map(|&x| blank(x)));
            i += len;
        } else if c == b'"' {
            out.push(b'"');
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    out.push(b' ');
                    i += 1;
                }
                if i < b.len() {
                    out.push(blank(b[i]));
                    i += 1;
                }
            }
            if i < b.len() {
                out.push(b'"');
                i += 1;
            }
        } else if c == b'\'' && is_char_literal(&b[i..]) {
            let len = char_literal_len(&b[i..]);
            out.extend(std::iter::repeat_n(b' ', len));
            i += len;
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The length of a raw string literal starting at `b[0] == b'r'`, if it is one.
pub fn raw_string_len(b: &[u8]) -> Option<usize> {
    let mut hashes = 0;
    let mut i = 1;
    while b.get(i) == Some(&b'#') {
        hashes += 1;
        i += 1;
    }
    if b.get(i) != Some(&b'"') {
        return None;
    }
    i += 1;
    while i < b.len() {
        if b[i] == b'"'
            && b.len() >= i + 1 + hashes
            && b[i + 1..i + 1 + hashes].iter().all(|&x| x == b'#')
        {
            return Some(i + 1 + hashes);
        }
        i += 1;
    }
    None
}

/// `'x'`, `'\n'`, `'\u{..}'` (a character literal) as opposed to a lifetime like `'a`.
pub fn is_char_literal(b: &[u8]) -> bool {
    match (b.get(1), b.get(2)) {
        (Some(b'\\'), _) => b[2..].iter().take(12).any(|&x| x == b'\''),
        (Some(_), Some(b'\'')) => true,
        _ => false,
    }
}

pub fn char_literal_len(b: &[u8]) -> usize {
    if b[1] == b'\\' {
        2 + b[2..].iter().position(|&x| x == b'\'').unwrap_or(0) + 1
    } else {
        3
    }
}

pub fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// `(name, body)` of every function with a body in `source` (already noise-blanked).
pub fn functions(source: &str) -> Vec<(String, String)> {
    function_spans(source)
        .into_iter()
        .map(|(name, start, end)| (name, source[start..end].to_owned()))
        .collect()
}

/// `(name, start, end)` byte span of the body of every function in (noise-blanked) `source`.
pub fn function_spans(source: &str) -> Vec<(String, usize, usize)> {
    let b = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 < b.len() {
        let is_fn = b[i] == b'f'
            && b[i + 1] == b'n'
            && b[i + 2].is_ascii_whitespace()
            && (i == 0 || !is_ident(b[i - 1]));
        if !is_fn {
            i += 1;
            continue;
        }
        let mut j = i + 3;
        while b.get(j).is_some_and(u8::is_ascii_whitespace) {
            j += 1;
        }
        let start = j;
        while b.get(j).is_some_and(|&c| is_ident(c)) {
            j += 1;
        }
        if j == start {
            i += 3;
            continue;
        }
        let name = source[start..j].to_owned();
        // The body starts at the first `{` outside parentheses; a `;` first means no body.
        let mut paren = 0_i32;
        let mut body_start = None;
        while j < b.len() {
            match b[j] {
                b'(' => paren += 1,
                b')' => paren -= 1,
                b';' if paren == 0 => break,
                b'{' if paren == 0 => {
                    body_start = Some(j);
                    break;
                }
                _ => {}
            }
            j += 1;
        }
        let Some(open) = body_start else {
            i = j.max(i + 3);
            continue;
        };
        let mut depth = 0_i32;
        let mut k = open;
        while k < b.len() {
            match b[k] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        out.push((name, open, k.min(b.len() - 1) + 1));
        i = open + 1;
    }
    out
}

pub fn count_calls(source: &str, name: &str) -> usize {
    let b = source.as_bytes();
    let n = name.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while let Some(at) = source[i..].find(name) {
        let start = i + at;
        let end = start + n.len();
        let before_ok = start == 0 || !is_ident(b[start - 1]);
        let mut after = end;
        while b.get(after).is_some_and(u8::is_ascii_whitespace) {
            after += 1;
        }
        let is_call =
            before_ok && b.get(end).is_none_or(|&c| !is_ident(c)) && b.get(after) == Some(&b'(');
        // `fn name(` is a definition, not a call.
        let is_definition = source[..start].trim_end().ends_with("fn");
        if is_call && !is_definition {
            count += 1;
        }
        i = end;
    }
    count
}

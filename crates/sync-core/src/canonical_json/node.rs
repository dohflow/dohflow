//! The restricted JSON value tree: no numbers, sorted ASCII member names.

use std::collections::BTreeMap;

use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

use super::error::CanonicalJsonError;

/// A value of the restricted JSON model of C.1.2: `null`, booleans, strings, arrays and
/// objects. There is no number type; every integer is a decimal string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<Node>),
    /// An object. Member names are ASCII `[a-z0-9_]`, so the map's byte order is also the
    /// UTF-16 code-unit order RFC 8785 requires.
    Object(BTreeMap<String, Node>),
}

/// Whether `name` is a legal member name: non-empty ASCII `[a-z0-9_]`.
#[must_use]
pub fn is_member_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The deepest nesting accepted, on both encoding and parsing: at most this many arrays and
/// objects inside one another (the outermost container is level 1). A second
/// implementation with a different limit would be a differential in its own right, so the
/// number is part of the format.
pub const MAX_DEPTH: usize = 128;

impl Node {
    /// Check the whole tree against the format's rules for a value that is about to be
    /// encoded: member names, no U+0000 in a string, and the nesting limit.
    pub(crate) fn validate(&self) -> Result<(), CanonicalJsonError> {
        self.validate_at(0)
    }

    fn validate_at(&self, containers: usize) -> Result<(), CanonicalJsonError> {
        match self {
            Node::Array(items) => {
                if containers >= MAX_DEPTH {
                    return Err(CanonicalJsonError::TooDeep(MAX_DEPTH));
                }
                items
                    .iter()
                    .try_for_each(|item| item.validate_at(containers + 1))
            }
            Node::Object(members) => {
                if containers >= MAX_DEPTH {
                    return Err(CanonicalJsonError::TooDeep(MAX_DEPTH));
                }
                for (name, value) in members {
                    if !is_member_name(name) {
                        return Err(CanonicalJsonError::BadMemberName(name.clone()));
                    }
                    value.validate_at(containers + 1)?;
                }
                Ok(())
            }
            Node::String(text) if text.contains('\u{0}') => {
                Err(CanonicalJsonError::Unsupported("U+0000 in a string"))
            }
            Node::Null | Node::Bool(_) | Node::String(_) => Ok(()),
        }
    }
}

/// Insert a member, enforcing the name rule and rejecting a duplicate.
pub(crate) fn insert_member(
    object: &mut BTreeMap<String, Node>,
    name: String,
    value: Node,
) -> Result<(), CanonicalJsonError> {
    if !is_member_name(&name) {
        return Err(CanonicalJsonError::BadMemberName(name));
    }
    if object.contains_key(&name) {
        return Err(CanonicalJsonError::DuplicateMember(name));
    }
    object.insert(name, value);
    Ok(())
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Node::Null => serializer.serialize_unit(),
            Node::Bool(b) => serializer.serialize_bool(*b),
            Node::String(s) => serializer.serialize_str(s),
            Node::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Node::Object(members) => {
                let mut map = serializer.serialize_map(Some(members.len()))?;
                for (name, value) in members {
                    map.serialize_entry(name, value)?;
                }
                map.end()
            }
        }
    }
}

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

impl Node {
    /// Check every object in the tree for legal member names.
    pub(crate) fn check_member_names(&self) -> Result<(), CanonicalJsonError> {
        match self {
            Node::Array(items) => items.iter().try_for_each(Node::check_member_names),
            Node::Object(members) => {
                for (name, value) in members {
                    if !is_member_name(name) {
                        return Err(CanonicalJsonError::BadMemberName(name.clone()));
                    }
                    value.check_member_names()?;
                }
                Ok(())
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

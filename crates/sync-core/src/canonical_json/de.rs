//! `Deserialize` out of a [`Node`]: integers are read from decimal strings with the exact
//! syntax `-?(0|[1-9][0-9]*)`; everything else is read as its JSON shape.
//!
//! Enums are externally tagged (`"unit"` or `{"variant": ...}`), which is the only tagging
//! that keeps integer-as-string decoding working: serde's internally and adjacently
//! tagged enums buffer their content and then read integers from the buffer as numbers.

use std::collections::btree_map;
use std::vec;

use serde::de::value::StringDeserializer;
use serde::de::{
    self, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess, SeqAccess,
    VariantAccess, Visitor,
};

use super::error::CanonicalJsonError;
use super::node::Node;

pub(crate) struct NodeDeserializer(pub(crate) Node);

/// Whether `s` matches `0|-?[1-9][0-9]*`: zero is spelled `0`, never `-0`, and there are no
/// leading zeros, so every integer has exactly one spelling.
pub(crate) fn valid_integer_syntax(s: &str) -> bool {
    let digits = s.strip_prefix('-').map_or(s, |rest| rest);
    match (s.starts_with('-'), digits.as_bytes()) {
        (false, [b'0']) => true,
        (_, [b'1'..=b'9', rest @ ..]) => rest.iter().all(u8::is_ascii_digit),
        _ => false,
    }
}

impl NodeDeserializer {
    fn integer<'de, V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CanonicalJsonError> {
        let Node::String(text) = self.0 else {
            return Err(CanonicalJsonError::Decode(super::error::Redacted::new(
                "expected an integer string",
            )));
        };
        if !valid_integer_syntax(&text) {
            return Err(CanonicalJsonError::BadInteger);
        }
        if text.starts_with('-') {
            let v: i128 = text
                .parse()
                .map_err(|_| CanonicalJsonError::IntegerOutOfRange)?;
            match i64::try_from(v) {
                Ok(small) => visitor.visit_i64(small),
                Err(_) => visitor.visit_i128(v),
            }
        } else {
            let v: u128 = text
                .parse()
                .map_err(|_| CanonicalJsonError::IntegerOutOfRange)?;
            match u64::try_from(v) {
                Ok(small) => visitor.visit_u64(small),
                Err(_) => visitor.visit_u128(v),
            }
        }
    }
}

impl<'de> Deserializer<'de> for NodeDeserializer {
    type Error = CanonicalJsonError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            Node::Null => visitor.visit_unit(),
            Node::Bool(b) => visitor.visit_bool(b),
            Node::String(s) => visitor.visit_string(s),
            Node::Array(items) => visitor.visit_seq(SeqDe(items.into_iter())),
            Node::Object(members) => visitor.visit_map(MapDe {
                iter: members.into_iter(),
                value: None,
            }),
        }
    }

    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.integer(visitor)
    }
    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.integer(visitor)
    }
    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.integer(visitor)
    }
    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.integer(visitor)
    }
    fn deserialize_i128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.integer(visitor)
    }
    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.integer(visitor)
    }
    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.integer(visitor)
    }
    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.integer(visitor)
    }
    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.integer(visitor)
    }
    fn deserialize_u128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.integer(visitor)
    }
    fn deserialize_f32<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
        Err(CanonicalJsonError::Float)
    }
    fn deserialize_f64<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
        Err(CanonicalJsonError::Float)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            Node::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.0 {
            Node::String(variant) => visitor.visit_enum(EnumDe {
                variant,
                value: None,
            }),
            Node::Object(members) if members.len() == 1 => {
                let (variant, value) = members.into_iter().next().expect("one member");
                visitor.visit_enum(EnumDe {
                    variant,
                    value: Some(value),
                })
            }
            _ => Err(CanonicalJsonError::Decode(super::error::Redacted::new(
                "expected a variant name or a single-member object",
            ))),
        }
    }

    serde::forward_to_deserialize_any! {
        bool char str string bytes byte_buf unit unit_struct seq tuple tuple_struct map struct
        identifier ignored_any
    }
}

struct SeqDe(vec::IntoIter<Node>);

impl<'de> SeqAccess<'de> for SeqDe {
    type Error = CanonicalJsonError;
    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        self.0
            .next()
            .map(|node| seed.deserialize(NodeDeserializer(node)))
            .transpose()
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.0.len())
    }
}

struct MapDe {
    iter: btree_map::IntoIter<String, Node>,
    value: Option<Node>,
}

impl<'de> MapAccess<'de> for MapDe {
    type Error = CanonicalJsonError;
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        match self.iter.next() {
            Some((key, value)) => {
                self.value = Some(value);
                // A key is read like any string value, so integer-keyed maps decode too.
                seed.deserialize(NodeDeserializer(Node::String(key)))
                    .map(Some)
            }
            None => Ok(None),
        }
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let node = self
            .value
            .take()
            .ok_or_else(|| de::Error::custom("a map value without a key"))?;
        seed.deserialize(NodeDeserializer(node))
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

struct EnumDe {
    variant: String,
    value: Option<Node>,
}

impl<'de> EnumAccess<'de> for EnumDe {
    type Error = CanonicalJsonError;
    type Variant = VariantDe;
    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), Self::Error> {
        let name: StringDeserializer<CanonicalJsonError> = self.variant.into_deserializer();
        let variant = seed.deserialize(name)?;
        Ok((variant, VariantDe(self.value)))
    }
}

struct VariantDe(Option<Node>);

impl<'de> VariantAccess<'de> for VariantDe {
    type Error = CanonicalJsonError;
    fn unit_variant(self) -> Result<(), Self::Error> {
        match self.0 {
            None | Some(Node::Null) => Ok(()),
            Some(_) => Err(de::Error::custom("a unit variant carries no value")),
        }
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, Self::Error> {
        match self.0 {
            Some(node) => seed.deserialize(NodeDeserializer(node)),
            None => Err(de::Error::custom("a newtype variant needs a value")),
        }
    }
    fn tuple_variant<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            Some(Node::Array(items)) => visitor.visit_seq(SeqDe(items.into_iter())),
            _ => Err(de::Error::custom("a tuple variant needs an array")),
        }
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.0 {
            Some(Node::Object(members)) => visitor.visit_map(MapDe {
                iter: members.into_iter(),
                value: None,
            }),
            _ => Err(de::Error::custom("a struct variant needs an object")),
        }
    }
}

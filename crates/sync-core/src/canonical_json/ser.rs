//! `Serialize` into a [`Node`]: every integer becomes its decimal string, floats and raw
//! bytes are refused, member names are validated.

use std::collections::BTreeMap;

use serde::ser::{
    Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer,
};

use super::error::CanonicalJsonError;
use super::node::{insert_member, Node};

pub(crate) struct NodeSerializer;

impl Serializer for NodeSerializer {
    type Ok = Node;
    type Error = CanonicalJsonError;
    type SerializeSeq = SeqSerializer;
    type SerializeTuple = SeqSerializer;
    type SerializeTupleStruct = SeqSerializer;
    type SerializeTupleVariant = VariantSeqSerializer;
    type SerializeMap = MapSerializer;
    type SerializeStruct = StructSerializer;
    type SerializeStructVariant = VariantStructSerializer;

    fn serialize_bool(self, v: bool) -> Result<Node, Self::Error> {
        Ok(Node::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Node, Self::Error> {
        Ok(Node::String(v.to_string()))
    }
    fn serialize_i16(self, v: i16) -> Result<Node, Self::Error> {
        Ok(Node::String(v.to_string()))
    }
    fn serialize_i32(self, v: i32) -> Result<Node, Self::Error> {
        Ok(Node::String(v.to_string()))
    }
    fn serialize_i64(self, v: i64) -> Result<Node, Self::Error> {
        Ok(Node::String(v.to_string()))
    }
    fn serialize_i128(self, v: i128) -> Result<Node, Self::Error> {
        Ok(Node::String(v.to_string()))
    }
    fn serialize_u8(self, v: u8) -> Result<Node, Self::Error> {
        Ok(Node::String(v.to_string()))
    }
    fn serialize_u16(self, v: u16) -> Result<Node, Self::Error> {
        Ok(Node::String(v.to_string()))
    }
    fn serialize_u32(self, v: u32) -> Result<Node, Self::Error> {
        Ok(Node::String(v.to_string()))
    }
    fn serialize_u64(self, v: u64) -> Result<Node, Self::Error> {
        Ok(Node::String(v.to_string()))
    }
    fn serialize_u128(self, v: u128) -> Result<Node, Self::Error> {
        Ok(Node::String(v.to_string()))
    }
    fn serialize_f32(self, _: f32) -> Result<Node, Self::Error> {
        Err(CanonicalJsonError::Float)
    }
    fn serialize_f64(self, _: f64) -> Result<Node, Self::Error> {
        Err(CanonicalJsonError::Float)
    }
    fn serialize_char(self, v: char) -> Result<Node, Self::Error> {
        self.serialize_str(v.encode_utf8(&mut [0; 4]))
    }
    fn serialize_str(self, v: &str) -> Result<Node, Self::Error> {
        if v.contains('\u{0}') {
            return Err(CanonicalJsonError::Unsupported("U+0000 in a string"));
        }
        Ok(Node::String(v.to_owned()))
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<Node, Self::Error> {
        Err(CanonicalJsonError::Unsupported("raw bytes"))
    }
    fn serialize_none(self) -> Result<Node, Self::Error> {
        Ok(Node::Null)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Node, Self::Error> {
        // `Some(x)` where `x` itself encodes as `null` (`Some(())`, `Some(None)`, a nested
        // `Option`) would encode exactly like `None`: the origin would apply "set to
        // nothing" while every replica decodes "leave alone". Refuse it at encode time.
        match value.serialize(self)? {
            Node::Null => Err(CanonicalJsonError::Unsupported(
                "an Option whose value encodes as null",
            )),
            node => Ok(node),
        }
    }
    fn serialize_unit(self) -> Result<Node, Self::Error> {
        Ok(Node::Null)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Node, Self::Error> {
        Ok(Node::Null)
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<Node, Self::Error> {
        Ok(Node::String(variant.to_owned()))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<Node, Self::Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Node, Self::Error> {
        let mut object = BTreeMap::new();
        insert_member(
            &mut object,
            variant.to_owned(),
            value.serialize(NodeSerializer)?,
        )?;
        Ok(Node::Object(object))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<SeqSerializer, Self::Error> {
        Ok(SeqSerializer(Vec::with_capacity(len.unwrap_or(0))))
    }
    fn serialize_tuple(self, len: usize) -> Result<SeqSerializer, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> Result<SeqSerializer, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<VariantSeqSerializer, Self::Error> {
        Ok(VariantSeqSerializer {
            variant,
            items: Vec::with_capacity(len),
        })
    }
    fn serialize_map(self, _: Option<usize>) -> Result<MapSerializer, Self::Error> {
        Ok(MapSerializer {
            object: BTreeMap::new(),
            pending_key: None,
        })
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<StructSerializer, Self::Error> {
        Ok(StructSerializer(BTreeMap::new()))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<VariantStructSerializer, Self::Error> {
        Ok(VariantStructSerializer {
            variant,
            object: BTreeMap::new(),
        })
    }
}

pub(crate) struct SeqSerializer(Vec<Node>);

impl SerializeSeq for SeqSerializer {
    type Ok = Node;
    type Error = CanonicalJsonError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.0.push(value.serialize(NodeSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Node, Self::Error> {
        Ok(Node::Array(self.0))
    }
}

impl SerializeTuple for SeqSerializer {
    type Ok = Node;
    type Error = CanonicalJsonError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<Node, Self::Error> {
        SerializeSeq::end(self)
    }
}

impl SerializeTupleStruct for SeqSerializer {
    type Ok = Node;
    type Error = CanonicalJsonError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<Node, Self::Error> {
        SerializeSeq::end(self)
    }
}

pub(crate) struct VariantSeqSerializer {
    variant: &'static str,
    items: Vec<Node>,
}

impl SerializeTupleVariant for VariantSeqSerializer {
    type Ok = Node;
    type Error = CanonicalJsonError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.items.push(value.serialize(NodeSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Node, Self::Error> {
        let mut object = BTreeMap::new();
        insert_member(
            &mut object,
            self.variant.to_owned(),
            Node::Array(self.items),
        )?;
        Ok(Node::Object(object))
    }
}

pub(crate) struct MapSerializer {
    object: BTreeMap<String, Node>,
    pending_key: Option<String>,
}

impl SerializeMap for MapSerializer {
    type Ok = Node;
    type Error = CanonicalJsonError;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        match key.serialize(NodeSerializer)? {
            Node::String(name) => {
                self.pending_key = Some(name);
                Ok(())
            }
            _ => Err(CanonicalJsonError::Unsupported(
                "a map key that is not a string",
            )),
        }
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        let name = self
            .pending_key
            .take()
            .ok_or(CanonicalJsonError::Unsupported("a map value without a key"))?;
        insert_member(&mut self.object, name, value.serialize(NodeSerializer)?)
    }
    fn end(self) -> Result<Node, Self::Error> {
        Ok(Node::Object(self.object))
    }
}

pub(crate) struct StructSerializer(BTreeMap<String, Node>);

impl SerializeStruct for StructSerializer {
    type Ok = Node;
    type Error = CanonicalJsonError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        insert_member(
            &mut self.0,
            key.to_owned(),
            value.serialize(NodeSerializer)?,
        )
    }
    fn end(self) -> Result<Node, Self::Error> {
        Ok(Node::Object(self.0))
    }
}

pub(crate) struct VariantStructSerializer {
    variant: &'static str,
    object: BTreeMap<String, Node>,
}

impl SerializeStructVariant for VariantStructSerializer {
    type Ok = Node;
    type Error = CanonicalJsonError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        insert_member(
            &mut self.object,
            key.to_owned(),
            value.serialize(NodeSerializer)?,
        )
    }
    fn end(self) -> Result<Node, Self::Error> {
        let mut outer = BTreeMap::new();
        insert_member(
            &mut outer,
            self.variant.to_owned(),
            Node::Object(self.object),
        )?;
        Ok(Node::Object(outer))
    }
}

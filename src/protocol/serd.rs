#![allow(unused)]
use serde::{
    Serializer,
    ser::{
        Impossible, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
        SerializeTuple, SerializeTupleStruct, SerializeTupleVariant,
    },
};
use std::fmt::Display;

#[derive(Debug, thiserror::Error, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum SerializeError {
    #[error("Failed to encode value")]
    EncodeError,
    #[error("Failed to serialize value: {0}")]
    Serialization(String),
}

impl serde::ser::Error for SerializeError {
    fn custom<T>(msg: T) -> Self
    where
        T: Display,
    {
        SerializeError::Serialization(msg.to_string())
    }
}

pub struct EsbuildSerializer {
    output: Vec<u8>,
}

impl EsbuildSerializer {
    fn write8(&mut self, b: u8) {
        self.output.push(b);
    }

    fn write_u32_raw(&mut self, n: u32) {
        self.output.extend(n.to_le_bytes())
    }

    fn write_length_prefixed_bytes(&mut self, b: &[u8]) {
        self.write_u32_raw(b.len() as u32);
        self.output.extend_from_slice(b);
    }
}

impl SerializeSeq for &'_ mut EsbuildSerializer {
    type Ok = ();

    type Error = SerializeError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl SerializeTuple for &'_ mut EsbuildSerializer {
    type Ok = ();

    type Error = SerializeError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl SerializeTupleStruct for &'_ mut EsbuildSerializer {
    type Ok = ();

    type Error = SerializeError;

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        value.serialize(&mut **self)
    }
}

impl SerializeTupleVariant for &'_ mut EsbuildSerializer {
    type Ok = ();

    type Error = SerializeError;

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        value.serialize(&mut **self)
    }
}

impl SerializeMap for &'_ mut EsbuildSerializer {
    type Ok = ();

    type Error = SerializeError;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        todo!()
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        todo!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        todo!()
    }
}

impl SerializeStruct for &mut EsbuildSerializer {
    type Ok = ();

    type Error = SerializeError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        todo!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        todo!()
    }
}

impl SerializeStructVariant for &mut EsbuildSerializer {
    type Ok = ();

    type Error = SerializeError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        todo!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        todo!()
    }
}

fn unexpected<O>(s: impl Display) -> Result<O, SerializeError> {
    Err(SerializeError::Serialization(format!("unexpected {}", s)))
}

macro_rules! unimpl {
    ($($t: ident),*) => {

        paste::paste! {
            $(
                fn [< serialize _ $t >](self, _v: $t) -> Result<Self::Ok, Self::Error> {
                    unexpected(stringify!($t))
                }
            )*
        }
    };
}

type ImpossibleT = Impossible<(), SerializeError>;

impl Serializer for &'_ mut EsbuildSerializer {
    type Ok = ();

    type Error = SerializeError;

    type SerializeSeq = Self;

    type SerializeTuple = Self;

    type SerializeTupleStruct = ImpossibleT;

    type SerializeTupleVariant = Impossible<(), SerializeError>;

    type SerializeMap = ImpossibleT;

    type SerializeStruct = ImpossibleT;

    type SerializeStructVariant = ImpossibleT;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        self.write8(1);
        self.write8(v as u8);
        todo!()
    }

    unimpl!(i8, i16, i32, i64, u8, u16, u64, f32, f64, char);

    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.write8(2);
        self.write_u32_raw(v);
        Ok(())
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        self.write8(3);
        self.write_length_prefixed_bytes(v.as_bytes());
        Ok(())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        self.write8(4);
        self.write_length_prefixed_bytes(v);
        Ok(())
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.write8(0);
        Ok(())
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.write8(0);
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.write8(0);
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        unexpected("unit variant")
    }

    fn serialize_newtype_struct<T>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        unexpected("newtype struct")
    }

    fn serialize_newtype_variant<T>(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        unexpected("newtype variant")
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        let len = len.unwrap();
        self.write_u32_raw(len as u32);
        Ok(self)
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.write_u32_raw(len as u32);
        Ok(self)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        unexpected("tuple struct")
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        unexpected("tuple variant")
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        todo!()
    }

    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        todo!()
    }

    fn serialize_struct_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        todo!()
    }
}

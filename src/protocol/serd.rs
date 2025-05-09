use serde::{
    Serializer,
    ser::{
        SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
        SerializeTupleStruct, SerializeTupleVariant,
    },
};
use std::fmt::Display;

#[derive(Debug, thiserror::Error, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
enum SerializeError {
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

pub struct EsbuildSerializer {}

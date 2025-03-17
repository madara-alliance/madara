use serde::{de, Deserializer, Serializer};
use serde_with::{DeserializeAs, SerializeAs};
use starknet_core::types::Hash256;
use std::fmt;

/// Bincode-friendly [`SerializeAs`] and [`DeserializeAs`] implementation for [`Hash256`]
pub struct Hash256Serde;

impl SerializeAs<Hash256> for Hash256Serde {
    fn serialize_as<S: Serializer>(value: &Hash256, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(value.as_bytes())
    }
}

impl<'de> DeserializeAs<'de, Hash256> for Hash256Serde {
    fn deserialize_as<D: Deserializer<'de>>(deserializer: D) -> Result<Hash256, D::Error> {
        struct Hash256Visitor;
        impl de::Visitor<'_> for Hash256Visitor {
            type Value = Hash256;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a byte array of size 32 representing a Hash256")
            }
            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                if v.len() != 32 {
                    return Err(de::Error::invalid_length(v.len(), &self));
                }
                let bytes: [u8; 32] = v.try_into().expect("condition checked just before");
                Ok(Hash256::from_bytes(bytes))
            }
        }
        deserializer.deserialize_bytes(Hash256Visitor)
    }
}

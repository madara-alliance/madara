use std::time::Duration;

use serde::{Deserialize, Deserializer};
use starknet_core::types::Felt;

use crate::{crypto::ZeroingPrivateKey, parsers::parse_duration};

pub fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_duration(&s).map_err(serde::de::Error::custom)
}

pub fn serialize_duration<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if duration.as_secs_f64().fract() == 0.0 {
        serializer.serialize_str(&format!("{}s", duration.as_secs()))
    } else {
        serializer.serialize_str(&format!("{}ms", duration.as_millis()))
    }
}

pub fn deserialize_private_key<'de, D>(deserializer: D) -> Result<ZeroingPrivateKey, D::Error>
where
    D: Deserializer<'de>,
{
    let mut private = Felt::deserialize(deserializer)?;
    Ok(ZeroingPrivateKey::new(&mut private))
}

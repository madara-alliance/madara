use std::time::Duration;

use serde::{Deserialize, Deserializer};
use starknet_types_core::felt::Felt;
use url::Url;

use crate::{crypto::ZeroingPrivateKey, parsers::parse_duration, parsers::parse_url};

pub fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_duration(&s).map_err(serde::de::Error::custom)
}

pub fn deserialize_optional_duration<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(s) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    parse_duration(&s).map_err(serde::de::Error::custom).map(Some)
}

pub fn deserialize_url<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_url(&s).map_err(serde::de::Error::custom)
}

pub fn serialize_optional_duration<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if let Some(duration) = duration {
        serialize_duration(duration, serializer)
    } else {
        serializer.serialize_none()
    }
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

pub fn serialize_url<S>(url: &Url, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(url.as_str())
}

pub fn deserialize_private_key<'de, D>(deserializer: D) -> Result<ZeroingPrivateKey, D::Error>
where
    D: Deserializer<'de>,
{
    let mut private = Felt::deserialize(deserializer)?;
    Ok(ZeroingPrivateKey::new(&mut private))
}

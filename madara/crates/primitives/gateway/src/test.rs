#[cfg(test)]
use std::fmt::Debug;

use serde::de::DeserializeOwned;

use super::*;

fn test_serialize_deserialize<T>(value: T) -> T
where
    T: Serialize + DeserializeOwned + Clone + Debug + PartialEq,
{
    let serialized = serde_json::to_string(&value).unwrap();
    let deserialized: T = serde_json::from_str(&serialized).unwrap();
    assert_eq!(value, deserialized);
    deserialized
}

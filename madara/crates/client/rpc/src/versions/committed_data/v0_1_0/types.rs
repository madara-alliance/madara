//! Bounded private RPC request and response values.
use super::MAX_DATASETS_PER_TRANSACTION;
use blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES;
use mp_convert::Felt;
use mp_gateway::committed_data::CommittedDataUpdate;
use serde::{Deserialize, Serialize};

/// Felt key/value pairs, bounded while decoding rather than after allocation.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct CommittedDataEntries(Vec<(Felt, Felt)>);

impl CommittedDataEntries {
    /// Consumes the bounded wire value without copying its dataset.
    pub fn into_inner(self) -> Vec<(Felt, Felt)> {
        self.0
    }
}

impl TryFrom<Vec<(Felt, Felt)>> for CommittedDataEntries {
    type Error = blockifier::execution::syscalls::committed_data::CommittedDataError;
    fn try_from(values: Vec<(Felt, Felt)>) -> Result<Self, Self::Error> {
        if values.is_empty() || values.len() > MAX_COMMITTED_DATA_VALUES {
            return Err(Self::Error::InvalidLength);
        }
        Ok(Self(values))
    }
}

impl<'de> Deserialize<'de> for CommittedDataEntries {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ValuesVisitor;
        impl<'de> serde::de::Visitor<'de> for ValuesVisitor {
            type Value = CommittedDataEntries;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "1..={MAX_COMMITTED_DATA_VALUES} committed-data entries")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<(Felt, Felt)>()? {
                    if values.len() == MAX_COMMITTED_DATA_VALUES {
                        return Err(serde::de::Error::custom("Too many committed-data entries"));
                    }
                    values.push(value);
                }
                if values.is_empty() {
                    return Err(serde::de::Error::custom("Empty committed-data dataset"));
                }
                Ok(CommittedDataEntries(values))
            }
        }
        deserializer.deserialize_seq(ValuesVisitor)
    }
}

/// One allowlisted Stark-curve signer. This authorizes storage, not publication of an Oracle root.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSignature {
    pub public_key: Felt,
    pub r: Felt,
    pub s: Felt,
}

/// One independently authenticated, immutable dataset to import before transaction submission.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedDataImport {
    /// If present, entries are upserts relative to this namespace/base; otherwise a full map.
    #[serde(default)]
    pub update: Option<CommittedDataUpdate>,
    pub root: Felt,
    pub entries: CommittedDataEntries,
    #[serde(default)]
    pub signatures: Vec<ImportSignature>,
}

/// At most four datasets are decoded for a single signed transaction.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct CommittedDataImports(pub Vec<CommittedDataImport>);

impl<'de> Deserialize<'de> for CommittedDataImports {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ImportsVisitor;
        impl<'de> serde::de::Visitor<'de> for ImportsVisitor {
            type Value = CommittedDataImports;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("1..=4 committed-data datasets")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
                let mut datasets = Vec::with_capacity(MAX_DATASETS_PER_TRANSACTION);
                loop {
                    if datasets.len() == MAX_DATASETS_PER_TRANSACTION {
                        if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                            return Err(serde::de::Error::custom("Too many committed-data datasets"));
                        }
                        break;
                    }
                    let Some(dataset) = sequence.next_element::<CommittedDataImport>()? else { break };
                    datasets.push(dataset);
                }
                if datasets.is_empty() {
                    return Err(serde::de::Error::custom("Empty committed-data dataset list"));
                }
                Ok(CommittedDataImports(datasets))
            }
        }
        deserializer.deserialize_seq(ImportsVisitor)
    }
}

/// Durable data and transaction admission. Poll normal receipts for execution and finality.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportAndSubmitResult {
    pub transaction_hash: Felt,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn committed_data_import_array_is_bounded_during_decoding() {
        assert!(serde_json::from_str::<CommittedDataEntries>("[]").is_err());
        let encoded = serde_json::to_string(&vec![(Felt::MAX, Felt::ZERO)]).unwrap();
        let entries: CommittedDataEntries = serde_json::from_str(&encoded).unwrap();
        assert_eq!(entries.0, vec![(Felt::MAX, Felt::ZERO)]);
        let count = blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES + 1;
        let encoded = format!("[{}]", std::iter::repeat_n("[\"0x0\",\"0x0\"]", count).collect::<Vec<_>>().join(","));
        assert!(serde_json::from_str::<CommittedDataEntries>(&encoded).is_err());
        let item = serde_json::json!({"root":Felt::ONE,"entries":[[Felt::ZERO,Felt::ONE]]});
        assert!(serde_json::from_value::<CommittedDataImports>(serde_json::json!([])).is_err());
        assert!(serde_json::from_value::<CommittedDataImports>(serde_json::json!(vec![item.clone(); 4])).is_ok());
        assert!(serde_json::from_value::<CommittedDataImports>(serde_json::json!(vec![item; 5])).is_err());
    }
}

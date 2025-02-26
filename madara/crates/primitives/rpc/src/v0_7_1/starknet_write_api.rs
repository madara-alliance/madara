use super::{BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn, TxnHash};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ClassAndTxnHash {
    pub class_hash: Felt,
    pub transaction_hash: TxnHash,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ContractAndTxnHash {
    pub contract_address: Felt,
    pub transaction_hash: TxnHash,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AddInvokeTransactionResult {
    pub transaction_hash: TxnHash,
}

/// Parameters of the `starknet_addInvokeTransaction` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddInvokeTransactionParams {
    /// The information needed to invoke the function (or account, for version 1 transactions)
    pub invoke_transaction: BroadcastedInvokeTxn,
}

impl Serialize for AddInvokeTransactionParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("invoke_transaction", &self.invoke_transaction)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for AddInvokeTransactionParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = AddInvokeTransactionParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_addInvokeTransaction`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let invoke_transaction: BroadcastedInvokeTxn =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 1 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(2, &"expected 1 parameters"));
                }

                Ok(AddInvokeTransactionParams { invoke_transaction })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    invoke_transaction: BroadcastedInvokeTxn,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(AddInvokeTransactionParams { invoke_transaction: helper.invoke_transaction })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_addDeclareTransaction` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddDeclareTransactionParams {
    /// Declare transaction required to declare a new class on Starknet
    pub declare_transaction: BroadcastedDeclareTxn,
}

impl Serialize for AddDeclareTransactionParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("declare_transaction", &self.declare_transaction)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for AddDeclareTransactionParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = AddDeclareTransactionParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_addDeclareTransaction`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let declare_transaction: BroadcastedDeclareTxn =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 1 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(2, &"expected 1 parameters"));
                }

                Ok(AddDeclareTransactionParams { declare_transaction })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    declare_transaction: BroadcastedDeclareTxn,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(AddDeclareTransactionParams { declare_transaction: helper.declare_transaction })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Parameters of the `starknet_addDeployAccountTransaction` method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddDeployAccountTransactionParams {
    /// The deploy account transaction
    pub deploy_account_transaction: BroadcastedDeployAccountTxn,
}

impl Serialize for AddDeployAccountTransactionParams {
    #[allow(unused_mut)]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("deploy_account_transaction", &self.deploy_account_transaction)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for AddDeployAccountTransactionParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = AddDeployAccountTransactionParams;

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "the parameters for `starknet_addDeployAccountTransaction`")
            }

            #[allow(unused_mut)]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let deploy_account_transaction: BroadcastedDeployAccountTxn =
                    seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &"expected 1 parameters"))?;

                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(2, &"expected 1 parameters"));
                }

                Ok(AddDeployAccountTransactionParams { deploy_account_transaction })
            }

            #[allow(unused_variables)]
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                #[derive(Deserialize)]
                struct Helper {
                    deploy_account_transaction: BroadcastedDeployAccountTxn,
                }

                let helper = Helper::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

                Ok(AddDeployAccountTransactionParams { deploy_account_transaction: helper.deploy_account_transaction })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

//! v0.8.1 of the API.
pub mod starknet_api_openrpc;
pub mod starknet_trace_api_openrpc;
pub mod starknet_ws_api;

pub use self::starknet_api_openrpc::*;
pub use self::starknet_trace_api_openrpc::*;
pub use self::starknet_ws_api::*;

impl From<crate::v0_7_1::BroadcastedTxn> for BroadcastedTxn {
    fn from(value: crate::v0_7_1::BroadcastedTxn) -> Self {
        match value {
            crate::v0_7_1::BroadcastedTxn::Invoke(tx) => Self::Invoke(tx.into()),
            crate::v0_7_1::BroadcastedTxn::Declare(tx) => Self::Declare(tx.into()),
            crate::v0_7_1::BroadcastedTxn::DeployAccount(tx) => Self::DeployAccount(tx.into()),
        }
    }
}

impl From<crate::v0_7_1::BroadcastedDeployAccountTxn> for BroadcastedDeployAccountTxn {
    fn from(value: crate::v0_7_1::BroadcastedDeployAccountTxn) -> Self {
        match value {
            crate::v0_7_1::BroadcastedDeployAccountTxn::V1(tx) => Self::V1(tx),
            crate::v0_7_1::BroadcastedDeployAccountTxn::V3(tx) => Self::V3(tx.into()),
            crate::v0_7_1::BroadcastedDeployAccountTxn::QueryV1(tx) => Self::QueryV1(tx),
            crate::v0_7_1::BroadcastedDeployAccountTxn::QueryV3(tx) => Self::QueryV3(tx.into()),
        }
    }
}

impl From<crate::v0_7_1::DeployAccountTxnV3> for DeployAccountTxnV3 {
    fn from(value: crate::v0_7_1::DeployAccountTxnV3) -> Self {
        Self {
            class_hash: value.class_hash,
            constructor_calldata: value.constructor_calldata,
            contract_address_salt: value.contract_address_salt,
            fee_data_availability_mode: value.fee_data_availability_mode,
            nonce: value.nonce,
            nonce_data_availability_mode: value.nonce_data_availability_mode,
            paymaster_data: value.paymaster_data,
            resource_bounds: value.resource_bounds.into(),
            signature: value.signature,
            tip: value.tip,
        }
    }
}

impl From<crate::v0_7_1::BroadcastedInvokeTxn> for BroadcastedInvokeTxn {
    fn from(value: crate::v0_7_1::BroadcastedInvokeTxn) -> Self {
        match value {
            crate::v0_7_1::BroadcastedInvokeTxn::V0(tx) => Self::V0(tx),
            crate::v0_7_1::BroadcastedInvokeTxn::V1(tx) => Self::V1(tx),
            crate::v0_7_1::BroadcastedInvokeTxn::V3(tx) => Self::V3(tx.into()),
            crate::v0_7_1::BroadcastedInvokeTxn::QueryV0(tx) => Self::QueryV0(tx),
            crate::v0_7_1::BroadcastedInvokeTxn::QueryV1(tx) => Self::QueryV1(tx),
            crate::v0_7_1::BroadcastedInvokeTxn::QueryV3(tx) => Self::QueryV3(tx.into()),
        }
    }
}

impl From<crate::v0_7_1::InvokeTxnV3> for InvokeTxnV3 {
    fn from(value: crate::v0_7_1::InvokeTxnV3) -> Self {
        Self {
            account_deployment_data: value.account_deployment_data,
            calldata: value.calldata,
            fee_data_availability_mode: value.fee_data_availability_mode,
            nonce: value.nonce,
            nonce_data_availability_mode: value.nonce_data_availability_mode,
            paymaster_data: value.paymaster_data,
            resource_bounds: value.resource_bounds.into(),
            sender_address: value.sender_address,
            signature: value.signature,
            tip: value.tip,
        }
    }
}

impl From<crate::v0_7_1::BroadcastedDeclareTxn> for BroadcastedDeclareTxn {
    fn from(value: crate::v0_7_1::BroadcastedDeclareTxn) -> Self {
        match value {
            crate::v0_7_1::BroadcastedDeclareTxn::V1(tx) => Self::V1(tx),
            crate::v0_7_1::BroadcastedDeclareTxn::V2(tx) => Self::V2(tx),
            crate::v0_7_1::BroadcastedDeclareTxn::V3(tx) => Self::V3(tx.into()),
            crate::v0_7_1::BroadcastedDeclareTxn::QueryV1(tx) => Self::QueryV1(tx),
            crate::v0_7_1::BroadcastedDeclareTxn::QueryV2(tx) => Self::QueryV2(tx),
            crate::v0_7_1::BroadcastedDeclareTxn::QueryV3(tx) => Self::QueryV3(tx.into()),
        }
    }
}

impl From<crate::v0_7_1::BroadcastedDeclareTxnV3> for BroadcastedDeclareTxnV3 {
    fn from(value: crate::v0_7_1::BroadcastedDeclareTxnV3) -> Self {
        Self {
            account_deployment_data: value.account_deployment_data,
            compiled_class_hash: value.compiled_class_hash,
            contract_class: value.contract_class,
            fee_data_availability_mode: value.fee_data_availability_mode,
            nonce: value.nonce,
            nonce_data_availability_mode: value.nonce_data_availability_mode,
            paymaster_data: value.paymaster_data,
            resource_bounds: value.resource_bounds.into(),
            sender_address: value.sender_address,
            signature: value.signature,
            tip: value.tip,
        }
    }
}

impl From<crate::v0_7_1::ResourceBoundsMapping> for ResourceBoundsMapping {
    fn from(value: crate::v0_7_1::ResourceBoundsMapping) -> Self {
        Self { l1_gas: value.l1_gas, l2_gas: value.l2_gas, l1_data_gas: Default::default() }
    }
}

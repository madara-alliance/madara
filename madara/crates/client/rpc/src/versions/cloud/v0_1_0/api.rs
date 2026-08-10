/// T-061: Cloud RPC API surface for Paradex Cloud.
///
/// Exposes `starknet_addInvokeTransaction` on the private cloud endpoint (port 9942).
/// Transactions are inserted directly into the mempool using the validated path,
/// bypassing full pre-validation.
use jsonrpsee::core::RpcResult;
use m_proc_macros::versioned_rpc;
use mp_rpc::v0_9_0::{AddInvokeTransactionResult, BroadcastedInvokeTxn};

#[versioned_rpc("V0_1_0", "starknet")]
pub trait ParadoxCloudWriteRpcApi {
    /// Submit an already-validated invoke transaction directly into the mempool.
    ///
    /// Skips full pre-validation (nonce check, fee check, stateful validation).
    /// Rejects query-only transactions. Returns the transaction hash on success.
    #[method(name = "addInvokeTransaction")]
    async fn add_validated_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult>;
}

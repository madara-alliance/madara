use crate::versions::user::v0_8_1::StarknetWriteRpcApiV0_8_1Server as V0_8_1Impl;
use crate::versions::user::v0_10_1::StarknetWriteRpcApiV0_10_1Server;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_rpc::v0_10_1::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};

// v0.10.1 write API implementation
// Main changes from v0.10.0:
// - BroadcastedInvokeTxnV3 now supports optional proof field (for gateway proof submission)
// - The proof should be passed to the gateway for transactions that include it
//
// Gateway proof handling (v0.10.1):
// When a transaction is submitted with a proof via addInvokeTransaction, the proof
// should be forwarded to the gateway. This is used for:
// - Verifying transaction validity before inclusion in a block
// - Supporting proof-of-validity schemes like SNARK proofs
//
// Current status: The proof field is defined in BroadcastedInvokeTxnV3 but not yet
// extracted or forwarded. Full implementation requires:
// 1. Using the v0.10.1 BroadcastedInvokeTxnV3 type instead of re-exported v0.10.0 type
// 2. Extracting the proof from INVOKE_TXN_V3 transactions
// 3. Forwarding the proof to the gateway client
// 4. Storing proof_facts with the transaction for later retrieval via INCLUDE_PROOF_FACTS

#[async_trait]
impl StarknetWriteRpcApiV0_10_1Server for Starknet {
    async fn add_declare_transaction(&self, declare_transaction: BroadcastedDeclareTxn) -> RpcResult<ClassAndTxnHash> {
        V0_8_1Impl::add_declare_transaction(self, declare_transaction).await
    }

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> RpcResult<ContractAndTxnHash> {
        V0_8_1Impl::add_deploy_account_transaction(self, deploy_account_transaction).await
    }

    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult> {
        // Note: v0.10.1 spec adds optional 'proof' field to BroadcastedInvokeTxnV3.
        // The proof field is not currently extracted or forwarded to the gateway.
        // When this is fully implemented:
        // 1. Extract proof from invoke_transaction if present
        // 2. Forward to gateway alongside the transaction
        // 3. Store proof_facts for later retrieval via INCLUDE_PROOF_FACTS response flag
        //
        // For now, transactions with proof will still be accepted and processed,
        // but the proof will be ignored (not forwarded to gateway).
        V0_8_1Impl::add_invoke_transaction(self, invoke_transaction).await
    }
}

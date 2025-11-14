use std::sync::Arc;

use alloy::network::Ethereum;
use alloy::primitives::{Bytes, I256, U256};
use alloy::providers::Provider;
use alloy::rpc::types::eth::TransactionReceipt;
use alloy::sol;
use alloy::transports::http::Http;
use alloy::transports::{RpcError, TransportErrorKind};
use async_trait::async_trait;

use crate::types::LocalWalletSignerMiddleware;

// TODO: should be moved to Zaun:
// https://github.com/keep-starknet-strange/zaun

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface StarknetValidityContract {
        function aggregatorProgramHash() external view returns (uint256);
        function cancelL1ToL2Message(uint256 toAddress, uint256 selector, uint256[] calldata payload, uint256 nonce) external returns (bytes32);
        function configHash() external view returns (uint256);
        function consumeMessageFromL2(uint256 fromAddress, uint256[] calldata payload) external returns (bytes32);
        function finalize() external;
        function getMaxL1MsgFee() external pure returns (uint256);
        function identify() external pure returns (string memory);
        function initialize(bytes calldata data) external;
        function isFinalized() external view returns (bool);
        function isFrozen() external view returns (bool);
        function isOperator(address user) external view returns (bool);
        function l1ToL2MessageCancellations(bytes32 msgHash) external view returns (uint256);
        function l1ToL2MessageNonce() external view returns (uint256);
        function l1ToL2Messages(bytes32 msgHash) external view returns (uint256);
        function l2ToL1Messages(bytes32 msgHash) external view returns (uint256);
        function messageCancellationDelay() external view returns (uint256);
        function programHash() external view returns (uint256);
        function registerOperator(address newOperator) external;
        function sendMessageToL2(uint256 toAddress, uint256 selector, uint256[] calldata payload) external payable returns (bytes32, uint256);
        function setAggregatorProgramHash(uint256 newAggregatorProgramHash) external;
        function setConfigHash(uint256 newConfigHash) external;
        function setMessageCancellationDelay(uint256 delayInSeconds) external;
        function setProgramHash(uint256 newProgramHash) external;
        function starknetAcceptGovernance() external;
        function starknetCancelNomination() external;
        function starknetIsGovernor(address user) external view returns (bool);
        function starknetNominateNewGovernor(address newGovernor) external;
        function starknetRemoveGovernor(address governorForRemoval) external;
        function startL1ToL2MessageCancellation(uint256 toAddress, uint256 selector, uint256[] calldata payload, uint256 nonce) external returns (bytes32);
        function stateBlockHash() external view returns (uint256);
        function stateBlockNumber() external view returns (int256);
        function stateRoot() external view returns (uint256);
        function unregisterOperator(address removedOperator) external;
        function updateState(uint256[] calldata programOutput, uint256 onchainDataHash, uint256 onchainDataSize) external;
        function updateStateKzgDA(uint256[] calldata programOutput, bytes[] calldata kzgProofs) external;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StarknetValidityContractError {
    #[error("RPC error: {0}")]
    RpcError(#[from] RpcError<TransportErrorKind>),
    #[error("Failed to estimate gas: {0}")]
    EstimateGasError(#[from] alloy::contract::Error),
    #[error("Pending transaction error: {0}")]
    PendingTransactionError(#[from] alloy::providers::PendingTransactionError),
}

#[async_trait]
pub trait StarknetValidityContractTrait {
    /// Retrieves the last block number settled
    async fn state_block_number(&self) -> Result<I256, alloy::contract::Error>;

    /// Update the L1 state
    async fn update_state(
        &self,
        program_output: Vec<U256>,
        onchain_data_hash: U256,
        onchain_data_size: U256,
    ) -> Result<TransactionReceipt, StarknetValidityContractError>;

    async fn update_state_kzg(
        &self,
        program_output: Vec<U256>,
        kzg_proof: [u8; 48],
    ) -> Result<TransactionReceipt, StarknetValidityContractError>;
}

#[async_trait]
impl<T> StarknetValidityContractTrait for T
where
    T: AsRef<StarknetValidityContract::StarknetValidityContractInstance<Arc<LocalWalletSignerMiddleware>, Ethereum>>
        + Send
        + Sync,
{
    async fn state_block_number(&self) -> Result<I256, alloy::contract::Error> {
        Ok(self.as_ref().stateBlockNumber().call().await?)
    }

    async fn update_state(
        &self,
        program_output: Vec<U256>,
        onchain_data_hash: U256,
        onchain_data_size: U256,
    ) -> Result<TransactionReceipt, StarknetValidityContractError> {
        let base_fee = self.as_ref().provider().as_ref().get_gas_price().await?;
        let from_address = self.as_ref().provider().as_ref().get_accounts().await?[0];
        let gas = self
            .as_ref()
            .updateState(program_output.clone(), onchain_data_hash, onchain_data_size)
            .from(from_address)
            .estimate_gas()
            .await?;
        let builder = self.as_ref().updateState(program_output, onchain_data_hash, onchain_data_size);
        builder
            .from(from_address)
            .nonce(2)
            .gas(gas)
            .gas_price(base_fee)
            .send()
            .await?
            .get_receipt()
            .await
            .map_err(StarknetValidityContractError::PendingTransactionError)
    }

    async fn update_state_kzg(
        &self,
        program_output: Vec<U256>,
        kzg_proof: [u8; 48],
    ) -> Result<TransactionReceipt, StarknetValidityContractError> {
        let base_fee = self.as_ref().provider().as_ref().get_gas_price().await?;
        let from_address = self.as_ref().provider().as_ref().get_accounts().await?[0];
        let proof_vec = vec![Bytes::from(kzg_proof.to_vec())];
        let gas = self
            .as_ref()
            .updateStateKzgDA(program_output.clone(), proof_vec.clone())
            .from(from_address)
            .estimate_gas()
            .await?;
        let builder = self.as_ref().updateStateKzgDA(program_output, proof_vec);
        builder
            .from(from_address)
            .nonce(2)
            .gas(gas)
            .gas_price(base_fee)
            .send()
            .await?
            .get_receipt()
            .await
            .map_err(StarknetValidityContractError::PendingTransactionError)
    }
}

use std::sync::Arc;

use async_trait::async_trait;

use alloy::{
    network::Ethereum,
    primitives::{I256, U256},
    providers::Provider,
    rpc::types::eth::TransactionReceipt,
    sol,
    transports::{http::Http, RpcError, TransportErrorKind},
};

use crate::types::LocalWalletSignerMiddleware;

// TODO: should be moved to Zaun:
// https://github.com/keep-starknet-strange/zaun

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface StarknetValidityContract {
        function setProgramHash(uint256 newProgramHash) external notFinalized onlyGovernance;
        function setConfigHash(uint256 newConfigHash) external notFinalized onlyGovernance;
        function setMessageCancellationDelay(uint256 delayInSeconds) external notFinalized onlyGovernance;

        function programHash() public view returns (uint256);
        function configHash() public view returns (uint256);

        function identify() external pure override returns (string memory);
        function stateRoot() external view returns (uint256);
        function stateBlockNumber() external view returns (int256);
        function stateBlockHash() external view returns (uint256);

        function updateState(uint256[] calldata programOutput, uint256 onchainDataHash, uint256 onchainDataSize) external onlyOperator;
        function updateStateKzgDA(uint256[] calldata programOutput, bytes calldata kzgProof) external onlyOperator;
    }
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
    ) -> Result<TransactionReceipt, RpcError<TransportErrorKind>>;

    async fn update_state_kzg(
        &self,
        program_output: Vec<U256>,
        kzg_proof: [u8; 48],
    ) -> Result<TransactionReceipt, RpcError<TransportErrorKind>>;
}

#[async_trait]
impl<T> StarknetValidityContractTrait for T
where
    T: AsRef<
            StarknetValidityContract::StarknetValidityContractInstance<
                Http<reqwest::Client>,
                Arc<LocalWalletSignerMiddleware>,
                Ethereum,
            >,
        > + Send
        + Sync,
{
    async fn state_block_number(&self) -> Result<I256, alloy::contract::Error> {
        Ok(self.as_ref().stateBlockNumber().call().await?._0)
    }

    async fn update_state(
        &self,
        program_output: Vec<U256>,
        onchain_data_hash: U256,
        onchain_data_size: U256,
    ) -> Result<TransactionReceipt, RpcError<TransportErrorKind>> {
        let base_fee = self.as_ref().provider().as_ref().get_gas_price().await.unwrap();
        let from_address = self.as_ref().provider().as_ref().get_accounts().await.unwrap()[0];
        let gas = self
            .as_ref()
            .updateState(program_output.clone(), onchain_data_hash, onchain_data_size)
            .from(from_address)
            .estimate_gas()
            .await
            .unwrap();
        let builder = self.as_ref().updateState(program_output, onchain_data_hash, onchain_data_size);
        builder.from(from_address).nonce(2).gas(gas).gas_price(base_fee).send().await.unwrap().get_receipt().await
    }

    async fn update_state_kzg(
        &self,
        program_output: Vec<U256>,
        kzg_proof: [u8; 48],
    ) -> Result<TransactionReceipt, RpcError<TransportErrorKind>> {
        let base_fee = self.as_ref().provider().as_ref().get_gas_price().await.unwrap();
        let from_address = self.as_ref().provider().as_ref().get_accounts().await.unwrap()[0];
        let gas = self
            .as_ref()
            .updateStateKzgDA(program_output.clone(), kzg_proof.into())
            .from(from_address)
            .estimate_gas()
            .await
            .unwrap();
        let builder = self.as_ref().updateStateKzgDA(program_output, kzg_proof.into());
        builder.from(from_address).nonce(2).gas(gas).gas_price(base_fee).send().await.unwrap().get_receipt().await
    }
}

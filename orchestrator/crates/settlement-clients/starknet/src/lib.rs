pub mod config;
pub mod conversion;
#[cfg(test)]
pub mod tests;

use std::sync::Arc;

use appchain_core_contract_client::clients::StarknetCoreContractClient;
use appchain_core_contract_client::interfaces::core_contract::CoreContract;
use async_trait::async_trait;
use color_eyre::eyre::{eyre, Context};
use color_eyre::Result;
use lazy_static::lazy_static;
use mockall::automock;
use mockall::predicate::*;
use orchestrator_settlement_client_interface::{SettlementClient, SettlementVerificationStatus};
use starknet::accounts::{ConnectedAccount, ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag, Felt, FunctionCall, TransactionExecutionStatus, U256};
use starknet::core::utils::get_selector_from_name;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use tokio::time::{sleep, Duration};

use crate::conversion::{slice_slice_u8_to_vec_field, slice_u8_to_field, u64_from_felt};

pub type LocalWalletSignerMiddleware = Arc<SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>>;

pub struct StarknetSettlementClient {
    pub account: LocalWalletSignerMiddleware,
    pub starknet_core_contract_client: StarknetCoreContractClient,
    pub core_contract_address: Felt,
    pub tx_finality_retry_delay_in_seconds: u64,
}

pub const ENV_ACCOUNT_ADDRESS: &str = "MADARA_ORCHESTRATOR_STARKNET_ACCOUNT_ADDRESS";
pub const ENV_PRIVATE_KEY: &str = "MADARA_ORCHESTRATOR_STARKNET_PRIVATE_KEY";

const MAX_RETRIES_VERIFY_TX_FINALITY: usize = 10;

use url::Url;
#[derive(Clone, Debug)]
pub struct StarknetSettlementValidatedArgs {
    pub starknet_rpc_url: Url,
    pub starknet_private_key: String,
    pub starknet_account_address: String,
    pub starknet_cairo_core_contract_address: String,
    pub starknet_finality_retry_wait_in_secs: u64,
}

// Assumed the contract called for settlement looks like:
// https://github.com/keep-starknet-strange/piltover

impl StarknetSettlementClient {
    pub async fn new_with_args(settlement_cfg: &StarknetSettlementValidatedArgs) -> Self {
        let provider: Arc<JsonRpcClient<HttpTransport>> =
            Arc::new(JsonRpcClient::new(HttpTransport::new(settlement_cfg.starknet_rpc_url.clone())));

        let signer_address = Felt::from_hex(&settlement_cfg.starknet_account_address).expect("Invalid signer address");

        let private_key = Felt::from_hex(&settlement_cfg.starknet_private_key).expect("Invalid private key");
        let signing_key = SigningKey::from_secret_scalar(private_key);
        let signer = LocalWallet::from(signing_key);

        let core_contract_address = Felt::from_hex(&settlement_cfg.starknet_cairo_core_contract_address)
            .expect("Invalid core contract address");

        let chain_id = provider.chain_id().await.expect("Failed to get chain id");

        let mut account =
            SingleOwnerAccount::new(provider.clone(), signer, signer_address, chain_id, ExecutionEncoding::New);

        // Set block ID to Pending like in the reference implementation
        account.set_block_id(BlockId::Tag(BlockTag::Pending));
        let account = Arc::new(account);

        let starknet_core_contract_client = StarknetCoreContractClient::new(core_contract_address, account.clone());

        StarknetSettlementClient {
            account,
            core_contract_address,
            starknet_core_contract_client,
            tx_finality_retry_delay_in_seconds: settlement_cfg.starknet_finality_retry_wait_in_secs,
        }
    }
}

lazy_static! {
    pub static ref CONTRACT_WRITE_UPDATE_STATE_SELECTOR: Felt =
        get_selector_from_name("update_state").expect("Invalid update state selector");
    // TODO: `stateBlockNumber` does not exists yet in our implementation:
    // https://github.com/keep-starknet-strange/piltover
    // It should get added to match the solidity implementation of the core contract.
    pub static ref CONTRACT_READ_STATE_BLOCK_NUMBER: Felt =
        get_selector_from_name("get_state").expect("Invalid update state selector");
}

// TODO: Note that we already have an implementation of the appchain core contract client available
// here: https://github.com/keep-starknet-strange/zaun/tree/main/crates/l3/appchain-core-contract-client
// However, this implementation uses different Felt types, and incorporating all of them
// into this repository would introduce unnecessary complexity.
// Therefore, we will wait for the update of starknet_rs in the Zaun repository before adapting
// the StarknetSettlementClient implementation.

#[automock]
#[async_trait]
impl SettlementClient for StarknetSettlementClient {
    /// Should register the proof on the base layer and return an external id
    /// which can be used to track the status.
    #[allow(unused)]
    async fn register_proof(&self, proof: [u8; 32]) -> Result<String> {
        !unimplemented!("register_proof not implemented yet")
    }

    /// Should be used to update state on core contract when DA is done in calldata
    async fn update_state_calldata(
        &self,
        snos_output: Vec<[u8; 32]>,
        program_output: Vec<[u8; 32]>,
        onchain_data_hash: [u8; 32],
        onchain_data_size: [u8; 32],
    ) -> Result<String> {
        tracing::info!(
            log_type = "starting",
            category = "update_state",
            function_type = "calldata",
            "Updating state with calldata."
        );
        let snos_output = slice_slice_u8_to_vec_field(snos_output.as_slice());
        let program_output = slice_slice_u8_to_vec_field(program_output.as_slice());
        let onchain_data_hash = slice_u8_to_field(&onchain_data_hash);
        let core_contract: &CoreContract = self.starknet_core_contract_client.as_ref();

        let low = u128::from_be_bytes(onchain_data_size[16..32].try_into()?);
        let high = u128::from_be_bytes(onchain_data_size[0..16].try_into()?);
        let size = U256::from_words(low, high);

        let invoke_result = core_contract
            .update_state(snos_output, program_output, onchain_data_hash, size)
            .await
            .map_err(|e| eyre!("Failed to update state with calldata: {:?}", e))?;
        tracing::info!(
            log_type = "completed",
            category = "update_state",
            function_type = "calldata",
            "State updated with calldata."
        );
        Ok(invoke_result.transaction_hash.to_hex_string())
    }

    /// Should verify the inclusion of a tx in the settlement layer
    async fn verify_tx_inclusion(&self, tx_hash: &str) -> Result<SettlementVerificationStatus> {
        tracing::info!(
            log_type = "starting",
            category = "verify_tx",
            function_type = "inclusion",
            tx_hash = %tx_hash,
            "Verifying tx inclusion."
        );
        let tx_hash = Felt::from_hex(tx_hash)?;
        let tx_receipt = self.account.provider().get_transaction_receipt(tx_hash).await?;
        let execution_result = tx_receipt.receipt.execution_result();
        let status = execution_result.status();

        match status {
            TransactionExecutionStatus::Reverted => {
                tracing::info!(
                    log_type = "completed",
                    category = "verify_tx",
                    tx_hash = %tx_hash,
                    function_type = "inclusion",
                    revert_reason = %execution_result.revert_reason().unwrap(),
                    "Tx inclusion verified."
                );
                Ok(SettlementVerificationStatus::Rejected(format!(
                    "Transaction {} has been reverted: {}",
                    tx_hash,
                    execution_result.revert_reason().unwrap_or_default()
                )))
            }
            TransactionExecutionStatus::Succeeded => {
                if tx_receipt.block.is_pending() {
                    tracing::info!(
                        log_type = "pending",
                        category = "verify_tx",
                        function_type = "inclusion",
                        tx_hash = %tx_hash,
                        "Tx inclusion pending."
                    );
                    Ok(SettlementVerificationStatus::Pending)
                } else {
                    tracing::info!(
                        log_type = "completed",
                        category = "verify_tx",
                        function_type = "inclusion",
                        tx_hash = %tx_hash,
                        "Tx inclusion verified."
                    );
                    Ok(SettlementVerificationStatus::Verified)
                }
            }
        }
    }

    /// Should be used to update state on core contract and publishing the blob simultaneously
    #[allow(unused)]
    async fn update_state_with_blobs(
        &self,
        program_output: Vec<[u8; 32]>,
        state_diff: Vec<Vec<u8>>,
        nonce: u64,
    ) -> Result<String> {
        !unimplemented!("not implemented yet.")
    }

    /// Wait for a pending tx to achieve finality
    async fn wait_for_tx_finality(&self, tx_hash: &str) -> Result<Option<u64>> {
        let mut retries = 0;
        let duration_to_wait_between_polling = Duration::from_secs(self.tx_finality_retry_delay_in_seconds);
        sleep(duration_to_wait_between_polling).await;

        let tx_hash = Felt::from_hex(tx_hash)?;
        loop {
            let tx_receipt = self.account.provider().get_transaction_receipt(tx_hash).await?;
            if tx_receipt.block.is_pending() {
                retries += 1;
                if retries > MAX_RETRIES_VERIFY_TX_FINALITY {
                    return Err(eyre!("Max retries exceeeded while waiting for tx {tx_hash} finality."));
                }
                sleep(duration_to_wait_between_polling).await;
            } else {
                break;
            }
        }

        let tx_receipt = self.account.provider().get_transaction_receipt(tx_hash).await?;
        Ok(tx_receipt.block.block_number())
    }

    /// Returns the last block settled from the core contract.
    async fn get_last_settled_block(&self) -> Result<Option<u64>> {
        let block_number: Vec<Felt> = self
            .account
            .provider()
            .call(
                FunctionCall {
                    contract_address: self.core_contract_address,
                    entry_point_selector: *CONTRACT_READ_STATE_BLOCK_NUMBER,
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Latest),
            )
            .await?;
        if block_number.is_empty() {
            return Err(eyre!("Could not fetch last block number from core contract."));
        }

        let last_block_number: Felt = block_number[1];

        // Why Felt::MAX ?
        // https://github.com/starkware-libs/cairo-lang/blob/a86e92bfde9c171c0856d7b46580c66e004922f3/src/starkware/starknet/solidity/StarknetState.sol#L19-L39
        // https://docs.rs/starknet-types-core/latest/starknet_types_core/felt/struct.Felt.html
        // When last_block_number is 0, then we return None, stating that no state update has happened yet.
        if last_block_number == Felt::MAX {
            return Ok(None);
        }

        let converted_value = u64_from_felt(block_number[1]).wrap_err("Failed to convert to u64")?;

        Ok(Some(converted_value))
    }

    /// Returns the nonce for the wallet in use.
    async fn get_nonce(&self) -> Result<u64> {
        let nonce = self.account.get_nonce().await?;
        let converted_value = u64_from_felt(nonce).wrap_err("Failed to convert to u64")?;
        Ok(converted_value)
    }
}

pub mod config;
pub mod conversion;

use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use lazy_static::lazy_static;
use mockall::{automock, predicate::*};
use starknet::accounts::ConnectedAccount;
use starknet::core::types::{ExecutionResult, MaybePendingTransactionReceipt};
use starknet::providers::Provider;
use starknet::{
    accounts::{Account, Call, ExecutionEncoding, SingleOwnerAccount},
    core::{
        types::{BlockId, BlockTag, FieldElement, FunctionCall},
        utils::get_selector_from_name,
    },
    providers::{jsonrpc::HttpTransport, JsonRpcClient},
    signers::{LocalWallet, SigningKey},
};
use tokio::time::{sleep, Duration};

use settlement_client_interface::{SettlementClient, SettlementVerificationStatus, SETTLEMENT_SETTINGS_NAME};
use utils::env_utils::get_env_var_or_panic;
use utils::settings::SettingsProvider;

use crate::config::StarknetSettlementConfig;
use crate::conversion::{slice_slice_u8_to_vec_field, slice_u8_to_field};

pub struct StarknetSettlementClient {
    pub account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    pub core_contract_address: FieldElement,
    pub tx_finality_retry_delay_in_seconds: u64,
}

pub const ENV_PUBLIC_KEY: &str = "STARKNET_PUBLIC_KEY";
pub const ENV_PRIVATE_KEY: &str = "STARKNET_PRIVATE_KEY";

const MAX_RETRIES_VERIFY_TX_FINALITY: usize = 10;

// Assumed the contract called for settlement l ooks like:
// https://github.com/keep-starknet-strange/piltover

impl StarknetSettlementClient {
    pub async fn with_settings(settings: &impl SettingsProvider) -> Self {
        let settlement_cfg: StarknetSettlementConfig = settings.get_settings(SETTLEMENT_SETTINGS_NAME).unwrap();
        let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(settlement_cfg.rpc_url)));

        let public_key = get_env_var_or_panic(ENV_PUBLIC_KEY);
        let signer_address = FieldElement::from_hex_be(&public_key).expect("invalid signer address");

        // TODO: Very insecure way of building the signer. Needs to be adjusted.
        let private_key = get_env_var_or_panic(ENV_PRIVATE_KEY);
        let signer = FieldElement::from_hex_be(&private_key).expect("Invalid private key");
        let signer = LocalWallet::from(SigningKey::from_secret_scalar(signer));

        let core_contract_address =
            FieldElement::from_hex_be(&settlement_cfg.core_contract_address).expect("Invalid core contract address");

        let account = SingleOwnerAccount::new(
            provider.clone(),
            signer,
            signer_address,
            provider.chain_id().await.unwrap(),
            ExecutionEncoding::Legacy,
        );

        StarknetSettlementClient {
            account,
            core_contract_address,
            tx_finality_retry_delay_in_seconds: settlement_cfg.tx_finality_retry_delay_in_seconds,
        }
    }
}

lazy_static! {
    pub static ref CONTRACT_WRITE_UPDATE_STATE_SELECTOR: FieldElement =
        get_selector_from_name("update_state").expect("Invalid update state selector");
    // TODO: `stateBlockNumber` does not exists yet in our implementation:
    // https://github.com/keep-starknet-strange/piltover
    // It should get added to match the solidity implementation of the core contract.
    pub static ref CONTRACT_READ_STATE_BLOCK_NUMBER: FieldElement =
        get_selector_from_name("stateBlockNumber").expect("Invalid update state selector");
}

// TODO: Note that we already have an implementation of the appchain core contract client available here:
// https://github.com/keep-starknet-strange/zaun/tree/main/crates/l3/appchain-core-contract-client
// However, this implementation uses different FieldElement types, and incorporating all of them
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
        program_output: Vec<[u8; 32]>,
        onchain_data_hash: [u8; 32],
        onchain_data_size: usize,
    ) -> Result<String> {
        let program_output = slice_slice_u8_to_vec_field(program_output.as_slice());
        let onchain_data_hash = slice_u8_to_field(&onchain_data_hash);
        let mut calldata: Vec<FieldElement> = Vec::with_capacity(program_output.len() + 2);
        calldata.extend(program_output);
        calldata.push(onchain_data_hash);
        calldata.push(FieldElement::from(onchain_data_size));
        let invoke_result = self
            .account
            .execute(vec![Call {
                to: self.core_contract_address,
                selector: *CONTRACT_WRITE_UPDATE_STATE_SELECTOR,
                calldata,
            }])
            .send()
            .await?;
        Ok(format!("0x{:x}", invoke_result.transaction_hash))
    }

    /// Should be used to update state on core contract when DA is in blobs/alt DA
    #[allow(unused)]
    async fn update_state_blobs(&self, program_output: Vec<[u8; 32]>, kzg_proof: [u8; 48]) -> Result<String> {
        !unimplemented!("not available for starknet settlement layer")
    }

    /// Should verify the inclusion of a tx in the settlement layer
    async fn verify_tx_inclusion(&self, tx_hash: &str) -> Result<SettlementVerificationStatus> {
        let tx_hash = FieldElement::from_hex_be(tx_hash)?;
        let tx_receipt = self.account.provider().get_transaction_receipt(tx_hash).await?;
        match tx_receipt {
            MaybePendingTransactionReceipt::Receipt(tx) => match tx.execution_result() {
                ExecutionResult::Succeeded => Ok(SettlementVerificationStatus::Verified),
                ExecutionResult::Reverted { reason } => {
                    Ok(SettlementVerificationStatus::Rejected(format!("Tx {} has been reverted: {}", tx_hash, reason)))
                }
            },
            MaybePendingTransactionReceipt::PendingReceipt(tx) => match tx.execution_result() {
                ExecutionResult::Succeeded => Ok(SettlementVerificationStatus::Pending),
                ExecutionResult::Reverted { reason } => Ok(SettlementVerificationStatus::Rejected(format!(
                    "Pending tx {} has been reverted: {}",
                    tx_hash, reason
                ))),
            },
        }
    }

    /// Wait for a pending tx to achieve finality
    async fn wait_for_tx_finality(&self, tx_hash: &str) -> Result<()> {
        let mut retries = 0;
        let duration_to_wait_between_polling = Duration::from_secs(self.tx_finality_retry_delay_in_seconds);
        sleep(duration_to_wait_between_polling).await;

        let tx_hash = FieldElement::from_hex_be(tx_hash)?;
        loop {
            let tx_receipt = self.account.provider().get_transaction_receipt(tx_hash).await?;
            if let MaybePendingTransactionReceipt::PendingReceipt(_) = tx_receipt {
                retries += 1;
                if retries > MAX_RETRIES_VERIFY_TX_FINALITY {
                    return Err(eyre!("Max retries exceeeded while waiting for tx {tx_hash} finality."));
                }
                sleep(duration_to_wait_between_polling).await;
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Returns the last block settled from the core contract.
    async fn get_last_settled_block(&self) -> Result<u64> {
        let block_number = self
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
        Ok(block_number[0].try_into()?)
    }
}

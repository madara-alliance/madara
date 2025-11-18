use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use alloy::consensus::{SignableTransaction, Signed, TxEip4844, TxEip4844Variant, TxEip4844WithSidecar};
#[cfg(not(feature = "testing"))]
use alloy::eips::eip2718::Encodable2718;
use alloy::eips::eip2930::AccessList;
use alloy::eips::eip4844::BYTES_PER_BLOB;
use alloy::eips::eip7594::BlobTransactionSidecarVariant;
use alloy::hex;
use alloy::network::{Ethereum, EthereumWallet};
use alloy::primitives::{Address, Bytes, B256, I256, U256};
use alloy::providers::{PendingTransactionBuilder, Provider, ProviderBuilder};
use alloy::rpc::types::TransactionReceipt;
use alloy::signers::local::PrivateKeySigner;
use async_trait::async_trait;
use c_kzg::{Blob, Bytes32, KzgProof, KzgSettings};
use color_eyre::eyre::{bail, eyre, Ok};
use color_eyre::Result;
use conversion::{get_input_data_for_eip_4844, prepare_sidecar};
use orchestrator_settlement_client_interface::{SettlementClient, SettlementVerificationStatus};
#[cfg(feature = "testing")]
use orchestrator_utils::env_utils::get_env_var_or_panic;
use url::Url;

use crate::clients::interfaces::validity_interface::StarknetValidityContractTrait;
use crate::clients::StarknetValidityContractClient;
use crate::conversion::{slice_u8_to_u256, vec_u8_32_to_vec_u256};
pub mod clients;
pub mod conversion;
mod error;
pub mod tests;
pub mod types;

use crate::error::SendTransactionError;
use crate::types::{bytes_be_to_u128, convert_stark_bigint_to_u256, DefaultHttpProvider};
use lazy_static::lazy_static;
use mockall::automock;
use tokio::time::sleep;
use tracing::{info, warn};

// For more details on state update, refer to the core contract logic
// https://github.com/starkware-libs/cairo-lang/blob/master/src/starkware/starknet/solidity/Output.sol

pub const ENV_PRIVATE_KEY: &str = "MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY";
pub const N_BLOBS_OFFSET: usize = 11;
pub const X_0_POINT_OFFSET: usize = 10; // =h(c, c') where c=f(p_i(tau)) and c'=poseidon_hash(state_diff)
pub const Y_LOW_POINT_OFFSET: usize = 11;
pub const Y_HIGH_POINT_OFFSET: usize = Y_LOW_POINT_OFFSET + 1;

// Ethereum Transaction Finality
const MAX_TX_FINALISATION_ATTEMPTS: usize = 30;
const REQUIRED_BLOCK_CONFIRMATIONS: u64 = 3;

// Ethereum Gas Price Estimation
const GAS_PRICE_MULTIPLIER_START: f64 = 1.2;
const GAS_PRICE_INCREMENT_PERCENTAGE: f64 = 1.5; // 50%
const GAS_PRICE_MIN_INCREMENT_PERCENTAGE: f64 = 1.1; // 10%
/// we noticed Starknet uses the same limit on the mainnet
/// https://etherscan.io/tx/0x8a58b936faaefb63ee1371991337ae3b99d74cb3504d73868615bf21fa2f25a1
const GAS_LIMIT_STATE_UPDATE: u64 = 5_500_000;

lazy_static! {
    pub static ref PROJECT_ROOT: PathBuf = PathBuf::from(format!("{}/../../../", env!("CARGO_MANIFEST_DIR")));
    pub static ref KZG_SETTINGS: KzgSettings = KzgSettings::load_trusted_setup_file(
        &PROJECT_ROOT.join("crates/settlement-clients/ethereum/src/trusted_setup.txt"),
        0 // precompute parameter: 0 for minimal memory usage
    )
    .expect("Error loading trusted setup file");
}

#[derive(Clone, Debug)]
pub struct EthereumSettlementValidatedArgs {
    pub ethereum_rpc_url: Url,

    pub ethereum_private_key: String,

    pub l1_core_contract_address: Address,

    pub starknet_operator_address: Address,

    pub ethereum_finality_retry_wait_in_secs: u64,

    pub max_gas_price_mul_factor: f64,

    pub disable_peerdas: bool,
}

pub struct EthereumSettlementClient {
    core_contract_client: StarknetValidityContractClient,
    wallet: EthereumWallet,
    wallet_address: Address,
    provider: Arc<DefaultHttpProvider>,
    impersonate_account: Option<Address>,
    tx_finality_retry_wait_in_seconds: u64,
    max_gas_price_mul_factor: f64,
    disable_peerdas: bool,
}

impl EthereumSettlementClient {
    pub fn new_with_args(settlement_cfg: &EthereumSettlementValidatedArgs) -> Self {
        let private_key = settlement_cfg.ethereum_private_key.clone();
        let signer: PrivateKeySigner = private_key.parse().expect("Failed to parse private key");
        let wallet_address = signer.address();
        let wallet = EthereumWallet::from(signer);

        // provider without wallet
        let provider = Arc::new(ProviderBuilder::new().connect_http(settlement_cfg.ethereum_rpc_url.clone()));

        // provider with wallet
        let filler_provider = Arc::new(
            ProviderBuilder::new().wallet(wallet.clone()).connect_http(settlement_cfg.ethereum_rpc_url.clone()),
        );

        let core_contract_client =
            StarknetValidityContractClient::new(settlement_cfg.l1_core_contract_address, filler_provider);

        EthereumSettlementClient {
            provider,
            core_contract_client,
            wallet,
            wallet_address,
            impersonate_account: None,
            tx_finality_retry_wait_in_seconds: settlement_cfg.ethereum_finality_retry_wait_in_secs,
            max_gas_price_mul_factor: settlement_cfg.max_gas_price_mul_factor,
            disable_peerdas: settlement_cfg.disable_peerdas,
        }
    }

    #[cfg(feature = "testing")]
    pub fn with_test_params(
        provider: DefaultHttpProvider,
        core_contract_address: Address,
        rpc_url: Url,
        impersonate_account: Option<Address>,
    ) -> Self {
        let private_key = get_env_var_or_panic(ENV_PRIVATE_KEY);
        let signer: PrivateKeySigner = private_key.parse().expect("Failed to parse private key");
        let wallet_address = signer.address();
        let wallet = EthereumWallet::from(signer);

        let fill_provider = Arc::new(ProviderBuilder::new().wallet(wallet.clone()).connect_http(rpc_url));

        let core_contract_client = StarknetValidityContractClient::new(core_contract_address, fill_provider);

        EthereumSettlementClient {
            provider: Arc::new(provider),
            core_contract_client,
            wallet,
            wallet_address,
            impersonate_account,
            max_gas_price_mul_factor: 2f64,
            tx_finality_retry_wait_in_seconds: 10,
            disable_peerdas: false, // for testing, default to sepolia/testnet behavior
        }
    }

    /// Build kzg proof for the x_0 point evaluation
    pub fn build_proof(
        n_blobs: u64,
        blob_data: Vec<Vec<u8>>,
        x_0_value: Bytes32,
        y_0_values_program_output: Vec<Bytes32>,
    ) -> Result<Vec<KzgProof>> {
        assert_eq!(blob_data.len(), n_blobs as usize);

        let mut kzg_proofs: Vec<KzgProof> = vec![];

        for i in 0..n_blobs {
            let fixed_size_blob: [u8; BYTES_PER_BLOB] = blob_data[i as usize].as_slice().try_into()?;

            let blob = Blob::new(fixed_size_blob);
            let commitment = KZG_SETTINGS.blob_to_kzg_commitment(&blob)?;
            let (kzg_proof, y_0_value) = KZG_SETTINGS.compute_kzg_proof(&blob, &x_0_value)?;

            let y_0_value_program_output = y_0_values_program_output[i as usize];

            if y_0_value != y_0_value_program_output {
                bail!(
                    "ERROR : y_0 value is different than expected. Expected {:?}, got {:?}",
                    y_0_value,
                    y_0_value_program_output
                );
            }

            // Verifying the proof for double check
            let eval =
                KZG_SETTINGS.verify_kzg_proof(&commitment.to_bytes(), &x_0_value, &y_0_value, &kzg_proof.to_bytes())?;

            if !eval {
                bail!("ERROR : Assertion failed, not able to verify the proof.");
            }

            kzg_proofs.push(kzg_proof);
        }

        Ok(kzg_proofs)
    }
}

#[automock]
#[async_trait]
impl SettlementClient for EthereumSettlementClient {
    /// Should register the proof on the base layer and return an external id
    /// which can be used to track the status.
    async fn register_proof(&self, _proof: [u8; 32]) -> Result<String> {
        unimplemented!("register_proof is not implemented for EthereumSettlementClient")
    }

    /// Should be used to update state on core contract when DA is done in calldata
    async fn update_state_calldata(
        &self,
        _snos_output: Vec<[u8; 32]>,
        program_output: Vec<[u8; 32]>,
        onchain_data_hash: [u8; 32],
        onchain_data_size: [u8; 32],
    ) -> Result<String> {
        info!(
            log_type = "starting",
            category = "update_state",
            function_type = "calldata",
            "Updating state with calldata."
        );
        let program_output: Vec<U256> = vec_u8_32_to_vec_u256(program_output.as_slice())?;
        let onchain_data_hash: U256 = slice_u8_to_u256(&onchain_data_hash)?;
        let onchain_data_size = U256::from_be_bytes(onchain_data_size);
        let tx_receipt =
            self.core_contract_client.update_state(program_output, onchain_data_hash, onchain_data_size).await?;
        info!(
            log_type = "completed",
            category = "update_state",
            function_type = "calldata",
            tx_hash = %tx_receipt.transaction_hash,
            "State updated with calldata."
        );
        Ok(format!("0x{:x}", tx_receipt.transaction_hash))
    }

    /// Should be used to update state on core contract when DA is in blobs/alt DA
    /// NOTE: state_diff is a vector of blobs (which in turn is a vector of u8)
    ///
    /// The following things are done:
    /// 1. Check if the current state in Ethereum is more than what the transaction is trying to
    /// 2. Send the transaction, retrying if the transaction is failing because of low gas price
    ///
    /// The transaction is retried when the transaction is rejected because a transaction with the
    /// same nonce is already in the mempool. In that case, we'll send more transactions with
    /// an increasing gas price multiplication factor. The multiplication factor is capped by
    /// `MADARA_ORCHESTRATOR_EIP1559_MAX_GAS_MUL_FACTOR` env variable.
    async fn update_state_with_blobs(
        &self,
        program_output: Vec<[u8; 32]>,
        state_diff: Vec<Vec<u8>>,
        _nonce: u64,
    ) -> Result<String> {
        info!(log_type = "starting", category = "update_state", function_type = "blobs", "Updating state with blobs.");

        let mut mul_factor = GAS_PRICE_MULTIPLIER_START;

        loop {
            let tx_envelope = self.create_transaction(program_output.clone(), state_diff.clone(), mul_factor).await?;
            let pending_transaction = match self.send_transaction(tx_envelope).await {
                Result::Ok(pending_transaction) => pending_transaction,
                Err(e) => match e {
                    SendTransactionError::ReplacementTransactionUnderpriced(e) => {
                        warn!(
                                "Failed to send the blob transaction: {:?} with {:?} multiplication factor, trying again...",
                                e, mul_factor
                            );
                        mul_factor = self.get_next_mul_factor(mul_factor)?;
                        continue;
                    }
                    SendTransactionError::Other(_) => {
                        bail!("Failed to send blob transaction: {:?}", e);
                    }
                },
            };

            tracing::info!(
                log_type = "completed",
                category = "update_state",
                function_type = "blobs",
                tx_type = if self.disable_peerdas { "blob_proofs" } else { "cell_proofs" },
                "State updated with blobs."
            );

            tracing::warn!("⏳ Waiting for txn finality...");

            // Waiting for transaction finality
            let res = self.wait_for_tx_finality(&pending_transaction.tx_hash().to_string()).await?;

            match res {
                Some(_) => {
                    tracing::info!("✅ Txn hash : {:?} finalized", pending_transaction.tx_hash().to_string());
                }
                None => {
                    tracing::error!("❌ Txn hash: {:?} not finalised", pending_transaction.tx_hash().to_string());
                }
            }
            return Ok(pending_transaction.tx_hash().to_string());
        }
    }

    /// Should verify the inclusion of a tx in the settlement layer
    async fn verify_tx_inclusion(&self, tx_hash: &str) -> Result<SettlementVerificationStatus> {
        info!(
            log_type = "starting",
            category = "verify_tx",
            function_type = "inclusion",
            tx_hash = %tx_hash,
            "Verifying tx inclusion."
        );
        let tx_hash = B256::from_str(tx_hash)?;
        let maybe_tx_status: Option<TransactionReceipt> = self.provider.get_transaction_receipt(tx_hash).await?;
        match maybe_tx_status {
            Some(tx_status) => {
                if tx_status.status() {
                    info!(
                        log_type = "completed",
                        category = "verify_tx",
                        function_type = "inclusion",
                        tx_hash = %tx_status.transaction_hash,
                        "Tx inclusion verified."
                    );
                    Ok(SettlementVerificationStatus::Verified)
                } else {
                    info!(
                        log_type = "pending",
                        category = "verify_tx",
                        function_type = "inclusion",
                        tx_hash = %tx_status.transaction_hash,
                        "Tx inclusion pending."
                    );
                    // TODO: Make sure that this is correct for other txn types as well
                    Ok(SettlementVerificationStatus::Rejected(format!(
                        "Txn {} of type {} rejected",
                        tx_hash,
                        tx_status.inner.tx_type()
                    )))
                }
            }
            None => {
                info!(
                    log_type = "pending",
                    category = "verify_tx",
                    function_type = "inclusion",
                    tx_hash = %tx_hash,
                    "Tx inclusion pending."
                );
                Ok(SettlementVerificationStatus::Pending)
            }
        }
    }

    /// Wait for a pending tx to achieve finality
    async fn wait_for_tx_finality(&self, tx_hash: &str) -> Result<Option<u64>> {
        for _ in 0..MAX_TX_FINALISATION_ATTEMPTS {
            if let Some(receipt) =
                self.provider.get_transaction_receipt(B256::from_str(tx_hash).expect("Unable to form")).await?
            {
                if let Some(block_number) = receipt.block_number {
                    let latest_block = self.provider.get_block_number().await?;
                    let confirmations = latest_block.saturating_sub(block_number);
                    if confirmations >= REQUIRED_BLOCK_CONFIRMATIONS {
                        return Ok(Some(block_number));
                    }
                }
            }
            // Defaults to 60 seconds
            sleep(Duration::from_secs(self.tx_finality_retry_wait_in_seconds)).await;
        }
        Ok(None)
    }

    /// Get the last block settled through the core contract
    async fn get_last_settled_block(&self) -> Result<Option<u64>> {
        let block_number = self.core_contract_client.state_block_number().await?;
        let minus_one = I256::from_str("-1")?;
        // Check if block_number is -1
        // Meaning that no state update has happened yet.
        if block_number == minus_one {
            return Ok(None);
        }

        // Convert to u64 and wrap in Some
        let value: u64 = block_number.try_into()?;
        Ok(Some(value))
    }

    async fn get_nonce(&self) -> Result<u64> {
        let nonce = self.provider.get_transaction_count(self.wallet_address).await?.to_string().parse()?;
        Ok(nonce)
    }
}

impl EthereumSettlementClient {
    /// Method to build the input bytes for a state update transaction
    pub async fn build_input_bytes(program_output: Vec<[u8; 32]>, state_diff: Vec<Vec<u8>>) -> Result<String> {
        let n_blobs = match program_output.get(N_BLOBS_OFFSET) {
            Some(n_blobs) => u64::from_be_bytes(n_blobs[24..32].try_into()?),
            None => bail!("Failed to get n_blobs from program output"),
        };

        if program_output.len() <= N_BLOBS_OFFSET + 2 * n_blobs as usize {
            bail!("Malformed program output");
        }

        let mut y_0_values: Vec<Bytes32> = vec![];
        for i in 0..n_blobs {
            y_0_values.push(Bytes32::from(
                convert_stark_bigint_to_u256(
                    bytes_be_to_u128(
                        program_output
                            .get(2 * (n_blobs as usize + i as usize) + 1 + Y_LOW_POINT_OFFSET)
                            .ok_or(eyre!("Malformed program output"))?,
                    ),
                    bytes_be_to_u128(
                        program_output
                            .get(2 * (n_blobs as usize + i as usize) + 1 + Y_HIGH_POINT_OFFSET)
                            .ok_or(eyre!("Malformed program output"))?,
                    ),
                )
                .to_be_bytes(),
            ));
        }

        let x_0_point = Bytes32::from_bytes(
            program_output.get(X_0_POINT_OFFSET).ok_or(eyre!("Malformed program output"))?.as_slice(),
        )
        .map_err(|e| eyre!("Failed to get x_0 point params: {}", e))?;

        let kzg_proofs = Self::build_proof(n_blobs, state_diff, x_0_point, y_0_values)
            .map_err(|e| eyre!("Failed to build KZG proofs: {}", e))?;

        // Convert Vec<KzgProof> to Vec<[u8; 48]>
        let kzg_proofs_bytes: Vec<[u8; 48]> =
            kzg_proofs.into_iter().map(|proof| proof.to_bytes().into_inner()).collect();

        Ok(get_input_data_for_eip_4844(program_output, kzg_proofs_bytes)?)
    }

    /// Method to create a blob transaction (pre-Fusaka, for mainnet)
    /// Creates transaction with blob proofs
    async fn create_transaction(
        &self,
        program_output: Vec<[u8; 32]>,
        state_diff: Vec<Vec<u8>>,
        mul_factor: f64,
    ) -> Result<Signed<TxEip4844Variant<BlobTransactionSidecarVariant>>> {
        // Prepare the sidecar based on the chain ID
        let sidecar = prepare_sidecar(&state_diff, &KZG_SETTINGS, self.disable_peerdas)?;

        // Get chain id and nonce for the transaction
        let chain_id: u64 = self.provider.get_chain_id().await?.to_string().parse()?;
        let nonce = self.provider.get_transaction_count(self.wallet_address).await?.to_string().parse()?;

        // Get gas price estimates with margin
        let (max_fee_per_gas, max_priority_fee_per_gas, max_fee_per_blob_gas) =
            self.get_gas_price_estimates(mul_factor).await?;

        // Prepare input bytes for transaction
        let input_bytes = Self::build_input_bytes(program_output, state_diff).await?;

        // Prepare EIP4844 transaction
        let tx = TxEip4844 {
            chain_id,
            nonce,
            gas_limit: GAS_LIMIT_STATE_UPDATE,
            max_fee_per_blob_gas,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to: self.core_contract_client.contract_address(),
            value: U256::from(0),
            access_list: AccessList(vec![]),
            blob_versioned_hashes: sidecar.versioned_hashes().collect(),
            input: Bytes::from(hex::decode(input_bytes)?),
        };

        // Add sidecar to transaction
        let tx_with_sidecar = TxEip4844WithSidecar { tx, sidecar: sidecar.clone() };
        let mut variant = TxEip4844Variant::from(tx_with_sidecar);
        // Sign transaction
        let signature = self.wallet.default_signer().sign_transaction(&mut variant).await?;
        Ok(variant.into_signed(signature))
    }

    async fn get_gas_price_estimates(&self, mul_factor: f64) -> Result<(u128, u128, u128)> {
        let eip1559_est = self.provider.estimate_eip1559_fees().await?;

        let max_fee_per_gas: u128 = self.add_safety_margin(eip1559_est.max_fee_per_gas, mul_factor);
        let max_priority_fee_per_gas: u128 = self.add_safety_margin(eip1559_est.max_priority_fee_per_gas, mul_factor);
        let max_fee_per_blob_gas: u128 = self.add_safety_margin(self.provider.get_blob_base_fee().await?, mul_factor);

        Ok((max_fee_per_gas, max_priority_fee_per_gas, max_fee_per_blob_gas))
    }

    // add a safety margin to the gas price to handle fluctuations
    fn add_safety_margin(&self, value: u128, mul_factor: f64) -> u128 {
        (value as f64 * mul_factor) as u128
    }

    fn get_next_mul_factor(&self, mul_factor: f64) -> Result<f64> {
        let min_mul_factor = GAS_PRICE_MIN_INCREMENT_PERCENTAGE * mul_factor;
        let max_mul_factor = GAS_PRICE_INCREMENT_PERCENTAGE * mul_factor;

        if min_mul_factor > self.max_gas_price_mul_factor {
            bail!("Gas price multiplier is too high")
        } else {
            Ok(self.max_gas_price_mul_factor.min(max_mul_factor))
        }
    }

    /// Method to send blob transaction (standard EIP4844)
    async fn send_transaction(
        &self,
        tx_envelope: Signed<TxEip4844Variant<BlobTransactionSidecarVariant>>,
    ) -> Result<PendingTransactionBuilder<Ethereum>, SendTransactionError> {
        // Sending transaction when testing
        #[cfg(feature = "testing")]
        let pending_transaction = {
            let txn_request = {
                test_config::configure_transaction(self.provider.clone(), tx_envelope, self.impersonate_account).await
            };
            self.provider.send_transaction(txn_request).await?
        };

        // Sending transaction when not testing
        #[cfg(not(feature = "testing"))]
        let pending_transaction = {
            let encoded = tx_envelope.encoded_2718();
            self.provider.send_raw_transaction(encoded.as_slice()).await.map_err(|e| {
                if e.to_string().contains("error code -32000: replacement transaction underpriced") {
                    warn!("Transaction rejected because of insufficient gas price");
                    SendTransactionError::ReplacementTransactionUnderpriced(e)
                } else {
                    SendTransactionError::Other(e)
                }
            })?
        };

        Result::Ok(pending_transaction)
    }
}

#[cfg(feature = "testing")]
mod test_config {
    use alloy::network::TransactionBuilder;
    use alloy::rpc::types::TransactionRequest;

    use super::*;

    #[allow(dead_code)]
    pub async fn configure_transaction(
        provider: Arc<DefaultHttpProvider>,
        tx_envelope: TxEnvelope,
        impersonate_account: Option<Address>,
    ) -> TransactionRequest {
        let mut txn_request: TransactionRequest = tx_envelope.into();

        // IMPORTANT to understand #[cfg(test)], #[cfg(not(test))] and SHOULD_IMPERSONATE_ACCOUNT
        // Two tests :  `update_state_blob_with_dummy_contract_works` &
        // `update_state_blob_with_impersonation_works` use a env var `SHOULD_IMPERSONATE_ACCOUNT` to inform
        // the function `update_state_with_blobs` about the kind of testing,
        // `SHOULD_IMPERSONATE_ACCOUNT` can have any of "0" or "1" value :
        //      - if "0" then : Testing via default Anvil address.
        //      - if "1" then : Testing via impersonating `Starknet Operator Address`.
        // Note : changing between "0" and "1" is handled automatically by each test function, `no` manual
        // change in `env.test` is needed.
        if let Some(impersonate_account) = impersonate_account {
            let nonce =
                provider.get_transaction_count(impersonate_account).await.unwrap().to_string().parse::<u64>().unwrap();
            txn_request.set_nonce(nonce);
            txn_request = txn_request.with_from(impersonate_account);
        }

        txn_request
    }
}

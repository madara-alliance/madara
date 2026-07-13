use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use alloy::consensus::{SignableTransaction, Signed, TxEip4844, TxEip4844Variant, TxEip4844WithSidecar};
#[cfg(not(feature = "testing"))]
use alloy::eips::eip2718::Encodable2718;
use alloy::eips::eip2930::AccessList;
use alloy::eips::eip4844::{BYTES_PER_BLOB, DATA_GAS_PER_BLOB};
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
use orchestrator_settlement_client_interface::{
    SettlementClient, SettlementVerificationStatus, StateUpdateTxAttempt, StateUpdateTxAttemptStatus,
    StateUpdateTxError, StateUpdateTxResult, MAX_BLOBS_PER_STATE_UPDATE,
};
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
use tokio::time::{sleep, Instant};
#[cfg(not(feature = "testing"))]
use tracing::warn;
use tracing::{debug, info};

// For more details on state update, refer to the core contract logic
// https://github.com/starkware-libs/cairo-lang/blob/master/src/starkware/starknet/solidity/Output.sol

pub const ENV_PRIVATE_KEY: &str = "MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY";
/// Conservative default maximum signed liability for one L2 Ethereum state-update transaction (0.01 ETH).
/// Operators prioritizing settlement liveness during high-fee periods should override this value.
pub const DEFAULT_L2_STATE_UPDATE_MAX_FEE_WEI: u128 = 10_000_000_000_000_000;
pub const N_BLOBS_OFFSET: usize = 11;
pub const X_0_POINT_OFFSET: usize = 10; // =h(c, c') where c=f(p_i(tau)) and c'=poseidon_hash(state_diff)
pub const Y_LOW_POINT_OFFSET: usize = 11;
pub const Y_HIGH_POINT_OFFSET: usize = Y_LOW_POINT_OFFSET + 1;

// Ethereum Transaction Finality
const MAX_TX_FINALISATION_ATTEMPTS: usize = 30;
const REQUIRED_BLOCK_CONFIRMATIONS: u64 = 3;

// Ethereum Gas Price Estimation
// For EIP-4844 blob transactions, blobpool requires a 100% price bump (2x) to replace a stuck transaction.
// See: https://github.com/ethereum/go-ethereum/blob/d0af257aa20fe9d3e244570ee4abb9a78ff3b9c4/core/txpool/blobpool/config.go#L34
// See: https://github.com/paradigmxyz/reth/blob/c2435ff6f8265088b9ded0014051c9a97d0d7b84/crates/transaction-pool/src/config.rs#L29
// See: https://github.com/NethermindEth/nethermind/blob/471bcb95bac677d2ffde5bb2e882e20186841b24/src/Nethermind/Nethermind.TxPool/Comparison/CompareReplacedBlobTx.cs#L40
// With 1.1x start, 2.0x increment, and the default max of 2 fee bumps: 1.1 -> 2.2 -> 4.4.
// The number of replacements is configurable via MADARA_ORCHESTRATOR_ETHEREUM_MAX_FEE_BUMPS.
const GAS_PRICE_MULTIPLIER_START: f64 = 1.1; // 10% above estimated gas price
const GAS_PRICE_INCREMENT_FACTOR: f64 = 2.0; // 2x multiplier (100% bump required for blob tx replacement)
const REPLACEMENT_FEE_BUMP_NUMERATOR: u128 = 21;
const REPLACEMENT_FEE_BUMP_DENOMINATOR: u128 = 10;
/// we noticed Starknet uses the same limit on the mainnet
/// https://etherscan.io/tx/0x8a58b936faaefb63ee1371991337ae3b99d74cb3504d73868615bf21fa2f25a1
const GAS_LIMIT_STATE_UPDATE: u64 = 5_500_000;

#[derive(Clone, Copy, Debug)]
struct StateUpdateFeeCaps {
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
    max_fee_per_blob_gas: u128,
}

struct PreparedStateUpdateTransaction {
    tx_envelope: Signed<TxEip4844Variant<BlobTransactionSidecarVariant>>,
    fee_caps: StateUpdateFeeCaps,
}

fn calculate_next_fee_bump_mul_factor(current_mul: f64, fee_bumps_used: u64, max_fee_bumps: u64) -> Option<f64> {
    if fee_bumps_used >= max_fee_bumps {
        None
    } else {
        Some(GAS_PRICE_INCREMENT_FACTOR * current_mul)
    }
}

fn format_multiplier(multiplier: f64) -> String {
    format!("{multiplier:.2}x")
}

fn format_tx_attempt(attempt: &StateUpdateTxAttempt, timeout_seconds: u64) -> String {
    let prefix = format!(
        "{}. {} attempted, nonce={}",
        attempt.attempt_no,
        format_multiplier(attempt.gas_multiplier),
        attempt.nonce
    );

    match attempt.status {
        StateUpdateTxAttemptStatus::Finalized => {
            format!("{prefix}, accepted with tx hash {}, finalized.", attempt.tx_hash.as_deref().unwrap_or("<missing>"))
        }
        StateUpdateTxAttemptStatus::Replaced | StateUpdateTxAttemptStatus::TimedOut => {
            format!(
                "{prefix}, accepted with tx hash {}, timed out after {timeout_seconds}s.",
                attempt.tx_hash.as_deref().unwrap_or("<missing>")
            )
        }
        StateUpdateTxAttemptStatus::RejectedUnderpriced => {
            format!(
                "{prefix}, rejected as underpriced, no hash{}.",
                attempt.error.as_deref().map(|error| format!(" ({error})")).unwrap_or_default()
            )
        }
        StateUpdateTxAttemptStatus::SubmissionFailed => {
            format!(
                "{prefix}, submission failed, no hash{}.",
                attempt.error.as_deref().map(|error| format!(" ({error})")).unwrap_or_default()
            )
        }
    }
}

fn format_tx_attempts(attempts: &[StateUpdateTxAttempt], timeout_seconds: u64) -> String {
    attempts.iter().map(|attempt| format_tx_attempt(attempt, timeout_seconds)).collect::<Vec<_>>().join("\n")
}

fn state_update_tx_error(
    attempts: &[StateUpdateTxAttempt],
    timeout_seconds: u64,
    next_multiplier: f64,
    fee_bumps_used: u64,
    max_fee_bumps: u64,
) -> StateUpdateTxError {
    StateUpdateTxError {
        message: format!(
            "State update transaction failed after {} fee attempts ({} / {} fee bumps used) over {}s confirmation windows.\nFee attempts:\n{}\nFailure: Next required bump would be {}, but fee bump retry budget is exhausted ({}/{}).",
            attempts.len(),
            fee_bumps_used,
            max_fee_bumps,
            timeout_seconds,
            format_tx_attempts(attempts, timeout_seconds),
            format_multiplier(next_multiplier),
            fee_bumps_used,
            max_fee_bumps
        ),
        attempts: attempts.to_vec(),
    }
}

fn state_update_replacement_preparation_error(
    attempts: &[StateUpdateTxAttempt],
    timeout_seconds: u64,
    error: &color_eyre::Report,
) -> StateUpdateTxError {
    StateUpdateTxError {
        message: format!(
            "State update replacement preparation failed after a prior transaction attempt. The existing transaction will be reconciled on job retry.\nPreparation failure: {error}\nFee attempts:\n{}",
            format_tx_attempts(attempts, timeout_seconds)
        ),
        attempts: attempts.to_vec(),
    }
}

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

    pub ethereum_tx_confirmation_timeout_secs: u64,

    pub ethereum_max_fee_bumps: u64,

    pub ethereum_l2_state_update_max_fee_wei: u128,

    pub disable_peerdas: bool,
}

pub struct EthereumSettlementClient {
    core_contract_client: StarknetValidityContractClient,
    wallet: EthereumWallet,
    wallet_address: Address,
    provider: Arc<DefaultHttpProvider>,
    #[allow(unused)]
    impersonate_account: Option<Address>,
    tx_finality_retry_wait_in_seconds: u64,
    tx_confirmation_timeout_seconds: u64,
    max_fee_bumps: u64,
    l2_state_update_max_fee_wei: u128,
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
            tx_confirmation_timeout_seconds: settlement_cfg.ethereum_tx_confirmation_timeout_secs,
            max_fee_bumps: settlement_cfg.ethereum_max_fee_bumps,
            l2_state_update_max_fee_wei: settlement_cfg.ethereum_l2_state_update_max_fee_wei,
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
            tx_finality_retry_wait_in_seconds: 10,
            tx_confirmation_timeout_seconds: 300,
            max_fee_bumps: 2,
            l2_state_update_max_fee_wei: DEFAULT_L2_STATE_UPDATE_MAX_FEE_WEI,
            disable_peerdas: true,
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
    /// an increasing gas price multiplication factor. The number of fee-bump replacements is capped by
    /// `MADARA_ORCHESTRATOR_ETHEREUM_MAX_FEE_BUMPS` env variable.
    async fn update_state_with_blobs(
        &self,
        program_output: Vec<[u8; 32]>,
        state_diff: Vec<Vec<u8>>,
        nonce: u64,
    ) -> Result<StateUpdateTxResult> {
        // TODO(prakhar,20/11/2025): Update the logs to add custom formatter - https://github.com/madara-alliance/madara/blob/d2a1e8050a3d01ccf398f57616cbc4fb6386aaa6/madara/crates/client/analytics/src/formatter.rs#L288
        info!(
            log_type = "starting",
            category = "update_state",
            state_diff_len = %state_diff.len(),
            program_output_len = %program_output.len(),
            "Updating state with blob"
        );

        let mut gas_multiplier = GAS_PRICE_MULTIPLIER_START;
        let mut attempt_no = 1_u64;
        let mut fee_bumps_used = 0_u64;
        let mut attempts = Vec::new();
        let mut previous_fee_caps = None;

        loop {
            debug!(
                attempt = attempt_no,
                nonce = nonce,
                gas_multiplier = %gas_multiplier,
                "Preparing transaction with gas multiplier"
            );

            let replacement_fee_floor = previous_fee_caps.map(Self::replacement_fee_floor);
            let prepared_transaction = match self
                .create_transaction(
                    program_output.clone(),
                    state_diff.clone(),
                    nonce,
                    gas_multiplier,
                    replacement_fee_floor,
                )
                .await
            {
                Result::Ok(transaction) => transaction,
                Result::Err(error) if attempts.is_empty() => return Err(error),
                Result::Err(error) => {
                    return Err(state_update_replacement_preparation_error(
                        &attempts,
                        self.tx_confirmation_timeout_seconds,
                        &error,
                    )
                    .into());
                }
            };
            let attempted_fee_caps = prepared_transaction.fee_caps;
            previous_fee_caps = Some(attempted_fee_caps);

            let pending_transaction = match self.send_transaction(prepared_transaction.tx_envelope).await {
                Result::Ok(pending_transaction) => pending_transaction,
                Result::Err(SendTransactionError::ReplacementTransactionUnderpriced(rpc_err)) => {
                    attempts.push(StateUpdateTxAttempt {
                        attempt_no,
                        tx_hash: None,
                        nonce,
                        gas_multiplier,
                        status: StateUpdateTxAttemptStatus::RejectedUnderpriced,
                        error: Some(rpc_err.to_string()),
                    });
                    match calculate_next_fee_bump_mul_factor(gas_multiplier, fee_bumps_used, self.max_fee_bumps) {
                        Some(next_mul_factor) => {
                            fee_bumps_used += 1;
                            info!(
                                attempt = attempt_no,
                                nonce = nonce,
                                next_multiplier = %next_mul_factor,
                                fee_bumps_used = fee_bumps_used,
                                max_fee_bumps = self.max_fee_bumps,
                                "Transaction underpriced, sending replacement transaction"
                            );
                            debug!(
                                current_multiplier = %gas_multiplier,
                                next_multiplier = %next_mul_factor,
                                error = ?rpc_err,
                                "Increasing gas multiplier for replacement transaction"
                            );
                            gas_multiplier = next_mul_factor;
                            attempt_no += 1;
                            continue;
                        }
                        None => {
                            let next_mul = GAS_PRICE_INCREMENT_FACTOR * gas_multiplier;
                            return Err(state_update_tx_error(
                                &attempts,
                                self.tx_confirmation_timeout_seconds,
                                next_mul,
                                fee_bumps_used,
                                self.max_fee_bumps,
                            )
                            .into());
                        }
                    }
                }
                Result::Err(e) => {
                    attempts.push(StateUpdateTxAttempt {
                        attempt_no,
                        tx_hash: None,
                        nonce,
                        gas_multiplier,
                        status: StateUpdateTxAttemptStatus::SubmissionFailed,
                        error: Some(e.to_string()),
                    });
                    return Err(StateUpdateTxError {
                        message: format!(
                            "State update transaction submission failed after {} fee attempts.\nFee attempts:\n{}",
                            attempts.len(),
                            format_tx_attempts(&attempts, self.tx_confirmation_timeout_seconds)
                        ),
                        attempts,
                    }
                    .into());
                }
            };

            info!(
                log_type = "completed",
                category = "update_state",
                function_type = "blobs",
                tx_type = if self.disable_peerdas { "blob_proofs" } else { "cell_proofs" },
                tx_hash = %pending_transaction.tx_hash(),
                attempt = attempt_no,
                nonce = nonce,
                gas_multiplier = %gas_multiplier,
                "State update transaction submitted to Ethereum with blobs"
            );

            let tx_hash = pending_transaction.tx_hash().to_string();
            let finalized_block = self
                .wait_for_tx_finality_until(&tx_hash, Duration::from_secs(self.tx_confirmation_timeout_seconds))
                .await?;

            if let Some(block_number) = finalized_block {
                attempts.push(StateUpdateTxAttempt {
                    attempt_no,
                    tx_hash: Some(tx_hash.clone()),
                    nonce,
                    gas_multiplier,
                    status: StateUpdateTxAttemptStatus::Finalized,
                    error: None,
                });
                info!(
                    tx_hash = %tx_hash,
                    attempt = attempt_no,
                    nonce = nonce,
                    gas_multiplier = %gas_multiplier,
                    finalized_block = block_number,
                    "Transaction finalized successfully"
                );
                return Ok(StateUpdateTxResult { tx_hash, attempts });
            }

            attempts.push(StateUpdateTxAttempt {
                attempt_no,
                tx_hash: Some(tx_hash.clone()),
                nonce,
                gas_multiplier,
                status: StateUpdateTxAttemptStatus::TimedOut,
                error: None,
            });

            if fee_bumps_used >= self.max_fee_bumps {
                let next_mul = GAS_PRICE_INCREMENT_FACTOR * gas_multiplier;
                return Err(state_update_tx_error(
                    &attempts,
                    self.tx_confirmation_timeout_seconds,
                    next_mul,
                    fee_bumps_used,
                    self.max_fee_bumps,
                )
                .into());
            }

            let next_mul_factor =
                calculate_next_fee_bump_mul_factor(gas_multiplier, fee_bumps_used, self.max_fee_bumps).ok_or_else(
                    || {
                        let next_mul = GAS_PRICE_INCREMENT_FACTOR * gas_multiplier;
                        state_update_tx_error(
                            &attempts,
                            self.tx_confirmation_timeout_seconds,
                            next_mul,
                            fee_bumps_used,
                            self.max_fee_bumps,
                        )
                    },
                )?;

            fee_bumps_used += 1;
            info!(
                tx_hash = %tx_hash,
                attempt = attempt_no,
                nonce = nonce,
                current_multiplier = %gas_multiplier,
                next_multiplier = %next_mul_factor,
                fee_bumps_used = fee_bumps_used,
                max_fee_bumps = self.max_fee_bumps,
                confirmation_timeout_seconds = self.tx_confirmation_timeout_seconds,
                "Transaction not finalized before timeout, sending fee-bump replacement"
            );
            gas_multiplier = next_mul_factor;
            attempt_no += 1;
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
                if !receipt.status() {
                    bail!("Transaction {} was rejected by the settlement layer", tx_hash);
                }

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
    async fn wait_for_tx_finality_until(&self, tx_hash: &str, timeout: Duration) -> Result<Option<u64>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(receipt) =
                self.provider.get_transaction_receipt(B256::from_str(tx_hash).expect("Unable to form")).await?
            {
                if !receipt.status() {
                    bail!("Transaction {} was rejected by the settlement layer", tx_hash);
                }

                if let Some(block_number) = receipt.block_number {
                    let latest_block = self.provider.get_block_number().await?;
                    let confirmations = latest_block.saturating_sub(block_number);
                    if confirmations >= REQUIRED_BLOCK_CONFIRMATIONS {
                        return Ok(Some(block_number));
                    }
                }
            }

            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }

            let sleep_duration = Duration::from_secs(self.tx_finality_retry_wait_in_seconds).min(deadline - now);
            sleep(sleep_duration).await;
        }
    }

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
        nonce: u64,
        mul_factor: f64,
        replacement_fee_floor: Option<StateUpdateFeeCaps>,
    ) -> Result<PreparedStateUpdateTransaction> {
        // Prepare the sidecar based on the chain ID
        let sidecar = prepare_sidecar(&state_diff, &KZG_SETTINGS, self.disable_peerdas)?;

        // Get chain id for the transaction. The nonce is pinned by the caller so fee-bump
        // replacements target the same pending transaction.
        let chain_id: u64 = self.provider.get_chain_id().await?.to_string().parse()?;

        // For replacement transactions, the multiplier applies to the previously attempted fee
        // caps through `replacement_fee_floor`. The fresh network estimate is kept at the normal
        // safety margin so replacements do not overpay by multiplying both paths.
        let estimate_mul_factor = if replacement_fee_floor.is_some() { GAS_PRICE_MULTIPLIER_START } else { mul_factor };
        let fee_caps = self.get_gas_price_estimates(estimate_mul_factor).await?;
        let fee_caps = replacement_fee_floor.map(|floor| Self::max_fee_caps(fee_caps, floor)).unwrap_or(fee_caps);
        let max_total_fee_wei =
            Self::ensure_l2_state_update_fee_within_cap(fee_caps, self.l2_state_update_max_fee_wei)?;
        debug!(
            nonce = nonce,
            gas_multiplier = %mul_factor,
            estimate_multiplier = %estimate_mul_factor,
            max_fee_per_gas = fee_caps.max_fee_per_gas,
            max_priority_fee_per_gas = fee_caps.max_priority_fee_per_gas,
            max_fee_per_blob_gas = fee_caps.max_fee_per_blob_gas,
            max_total_fee_wei = %max_total_fee_wei,
            max_total_fee_cap_wei = self.l2_state_update_max_fee_wei,
            replacement_fee_floor = ?replacement_fee_floor,
            "Resolved state update transaction fee caps"
        );

        // Prepare input bytes for transaction
        let input_bytes = Self::build_input_bytes(program_output, state_diff).await?;

        // Prepare EIP4844 transaction
        let tx = TxEip4844 {
            chain_id,
            nonce,
            gas_limit: GAS_LIMIT_STATE_UPDATE,
            max_fee_per_blob_gas: fee_caps.max_fee_per_blob_gas,
            max_fee_per_gas: fee_caps.max_fee_per_gas,
            max_priority_fee_per_gas: fee_caps.max_priority_fee_per_gas,
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
        Ok(PreparedStateUpdateTransaction { tx_envelope: variant.into_signed(signature), fee_caps })
    }

    async fn get_gas_price_estimates(&self, mul_factor: f64) -> Result<StateUpdateFeeCaps> {
        let eip1559_est = self.provider.estimate_eip1559_fees().await?;

        let max_fee_per_gas: u128 = self.add_safety_margin(eip1559_est.max_fee_per_gas, mul_factor);
        let max_priority_fee_per_gas: u128 = self.add_safety_margin(eip1559_est.max_priority_fee_per_gas, mul_factor);
        let max_fee_per_blob_gas: u128 = self.add_safety_margin(self.provider.get_blob_base_fee().await?, mul_factor);

        Ok(StateUpdateFeeCaps { max_fee_per_gas, max_priority_fee_per_gas, max_fee_per_blob_gas })
    }

    // add a safety margin to the gas price to handle fluctuations
    fn add_safety_margin(&self, value: u128, mul_factor: f64) -> u128 {
        (value as f64 * mul_factor).ceil() as u128
    }

    fn replacement_fee_floor(previous_fee_caps: StateUpdateFeeCaps) -> StateUpdateFeeCaps {
        StateUpdateFeeCaps {
            max_fee_per_gas: Self::replacement_fee_cap_floor(previous_fee_caps.max_fee_per_gas),
            max_priority_fee_per_gas: Self::replacement_fee_cap_floor(previous_fee_caps.max_priority_fee_per_gas),
            max_fee_per_blob_gas: Self::replacement_fee_cap_floor(previous_fee_caps.max_fee_per_blob_gas),
        }
    }

    fn replacement_fee_cap_floor(previous_fee_cap: u128) -> u128 {
        previous_fee_cap
            .checked_mul(REPLACEMENT_FEE_BUMP_NUMERATOR)
            .map(|value| value.div_ceil(REPLACEMENT_FEE_BUMP_DENOMINATOR))
            .unwrap_or(u128::MAX)
    }

    fn max_fee_caps(left: StateUpdateFeeCaps, right: StateUpdateFeeCaps) -> StateUpdateFeeCaps {
        StateUpdateFeeCaps {
            max_fee_per_gas: left.max_fee_per_gas.max(right.max_fee_per_gas),
            max_priority_fee_per_gas: left.max_priority_fee_per_gas.max(right.max_priority_fee_per_gas),
            max_fee_per_blob_gas: left.max_fee_per_blob_gas.max(right.max_fee_per_blob_gas),
        }
    }

    fn ensure_l2_state_update_fee_within_cap(fee_caps: StateUpdateFeeCaps, max_fee_wei: u128) -> Result<U256> {
        let execution_fee = U256::from(GAS_LIMIT_STATE_UPDATE) * U256::from(fee_caps.max_fee_per_gas);
        let blob_fee = U256::from(MAX_BLOBS_PER_STATE_UPDATE)
            * U256::from(DATA_GAS_PER_BLOB)
            * U256::from(fee_caps.max_fee_per_blob_gas);
        let total_fee = execution_fee + blob_fee;

        if total_fee > U256::from(max_fee_wei) {
            bail!(
                "L2 state update signed fee liability {} wei exceeds configured hard cap {} wei",
                total_fee,
                max_fee_wei
            );
        }

        Ok(total_fee)
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
        tx_envelope: Signed<TxEip4844Variant<BlobTransactionSidecarVariant>>,
        impersonate_account: Option<Address>,
    ) -> TransactionRequest {
        // Extract the base transaction from the variant and convert to TransactionRequest
        // For testing, we convert the variant to a standard TransactionRequest
        let mut txn_request: TransactionRequest = match tx_envelope.tx() {
            TxEip4844Variant::TxEip4844(_) => {
                panic!("Wrong transaction type")
            }
            TxEip4844Variant::TxEip4844WithSidecar(tx_with_sidecar) => {
                let sidecar = match &tx_with_sidecar.sidecar {
                    BlobTransactionSidecarVariant::Eip4844(sidecar) => sidecar,
                    BlobTransactionSidecarVariant::Eip7594(_) => {
                        panic!("Wrong sidecar type")
                    }
                };
                let tx = TxEip4844WithSidecar { tx: tx_with_sidecar.tx.clone(), sidecar: sidecar.clone() };
                match tx_with_sidecar {
                    &_ => {}
                }
                <TransactionRequest as From<TxEip4844WithSidecar>>::from(tx)
            }
        };

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

#[cfg(test)]
mod gas_multiplier_tests {
    use super::*;

    #[test]
    fn test_fee_bump_policy_allows_two_bumps_to_four_point_four() {
        let first_bump = calculate_next_fee_bump_mul_factor(1.1, 0, 2).expect("first bump should fit policy");
        assert!((first_bump - 2.2).abs() < 0.0001, "Expected 2.2, got {}", first_bump);

        let second_bump = calculate_next_fee_bump_mul_factor(first_bump, 1, 2).expect("second bump should fit policy");
        assert!((second_bump - 4.4).abs() < 0.0001, "Expected 4.4, got {}", second_bump);
    }

    #[test]
    fn test_fee_bump_policy_stops_after_max_bumps() {
        let result = calculate_next_fee_bump_mul_factor(4.4, 2, 2);
        assert!(result.is_none(), "Expected None when fee bump budget is exhausted");
    }
}

#[cfg(test)]
mod fee_cap_tests {
    use super::*;

    #[test]
    fn l2_state_update_fee_cap_is_inclusive_and_assumes_six_blobs() {
        let fee_caps = StateUpdateFeeCaps {
            max_fee_per_gas: 1,
            // Priority fee is already included in max_fee_per_gas.
            max_priority_fee_per_gas: u128::MAX,
            max_fee_per_blob_gas: 1,
        };
        let expected_fee =
            U256::from(GAS_LIMIT_STATE_UPDATE) + U256::from(MAX_BLOBS_PER_STATE_UPDATE) * U256::from(DATA_GAS_PER_BLOB);
        let exact_cap = expected_fee.to::<u128>();

        assert_eq!(
            EthereumSettlementClient::ensure_l2_state_update_fee_within_cap(fee_caps, exact_cap).unwrap(),
            expected_fee
        );

        let error =
            EthereumSettlementClient::ensure_l2_state_update_fee_within_cap(fee_caps, exact_cap - 1).unwrap_err();
        assert!(error.to_string().contains("exceeds configured hard cap"));
    }

    #[test]
    fn replacement_preparation_error_preserves_pending_attempt_for_job_retry() {
        let attempts = vec![StateUpdateTxAttempt {
            attempt_no: 1,
            tx_hash: Some("0xabc".to_string()),
            nonce: 7,
            gas_multiplier: GAS_PRICE_MULTIPLIER_START,
            status: StateUpdateTxAttemptStatus::TimedOut,
            error: None,
        }];

        let source = eyre!("signed fee liability exceeds configured hard cap");
        let error = state_update_replacement_preparation_error(&attempts, 300, &source);

        assert_eq!(error.attempts, attempts);
        assert!(error.message.contains("reconciled on job retry"));
        assert!(error.message.contains("0xabc"));
        assert!(error.message.contains("signed fee liability exceeds configured hard cap"));
    }
}

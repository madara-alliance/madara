pub mod clients;
pub mod config;
pub mod conversion;
pub mod types;

use alloy::consensus::{
    BlobTransactionSidecar, SignableTransaction, TxEip4844, TxEip4844Variant, TxEip4844WithSidecar, TxEnvelope,
};
use alloy::eips::eip2718::Encodable2718;
use alloy::eips::eip2930::AccessList;
use alloy::eips::eip4844::BYTES_PER_BLOB;
use alloy::primitives::{Bytes, FixedBytes};
use alloy::{
    network::EthereumWallet,
    primitives::{Address, B256, U256},
    providers::{PendingTransactionConfig, Provider, ProviderBuilder},
    rpc::types::TransactionReceipt,
    signers::local::PrivateKeySigner,
};
use async_trait::async_trait;
use c_kzg::{Blob, Bytes32, KzgCommitment, KzgProof, KzgSettings};
use color_eyre::eyre::eyre;
use color_eyre::Result;
use mockall::{automock, lazy_static, predicate::*};
use rstest::rstest;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use crate::clients::interfaces::validity_interface::StarknetValidityContractTrait;
use settlement_client_interface::{SettlementClient, SettlementVerificationStatus, SETTLEMENT_SETTINGS_NAME};
use utils::{env_utils::get_env_var_or_panic, settings::SettingsProvider};

use crate::clients::StarknetValidityContractClient;
use crate::config::EthereumSettlementConfig;
use crate::conversion::{slice_slice_u8_to_vec_u256, slice_u8_to_u256};
use crate::types::EthHttpProvider;

pub const ENV_PRIVATE_KEY: &str = "ETHEREUM_PRIVATE_KEY";

lazy_static! {
    pub static ref CURRENT_PATH: PathBuf = std::env::current_dir().unwrap();
    pub static ref KZG_SETTINGS: KzgSettings = KzgSettings::load_trusted_setup_file(
        CURRENT_PATH.join("../../../orchestrator/src/jobs/state_update_job/trusted_setup.txt").as_path()
    )
    .expect("Error loading trusted setup file");
}

#[allow(dead_code)]
pub struct EthereumSettlementClient {
    provider: Arc<EthHttpProvider>,
    core_contract_client: StarknetValidityContractClient,
    wallet: EthereumWallet,
    wallet_address: Address,
}

impl EthereumSettlementClient {
    pub fn with_settings(settings: &impl SettingsProvider) -> Self {
        let settlement_cfg: EthereumSettlementConfig = settings.get_settings(SETTLEMENT_SETTINGS_NAME).unwrap();

        let private_key = get_env_var_or_panic(ENV_PRIVATE_KEY);
        let signer: PrivateKeySigner = private_key.parse().expect("Failed to parse private key");
        let wallet = EthereumWallet::from(signer.clone());

        let wallet_address = signer.address();

        let provider = Arc::new(
            ProviderBuilder::new().with_recommended_fillers().wallet(wallet.clone()).on_http(settlement_cfg.rpc_url),
        );
        let core_contract_client = StarknetValidityContractClient::new(
            Address::from_slice(settlement_cfg.core_contract_address.as_bytes()).0.into(),
            provider.clone(),
        );

        EthereumSettlementClient { provider, core_contract_client, wallet, wallet_address }
    }

    /// Build kzg proof for the x_0 point evaluation
    async fn build_proof(blob_data: Vec<Vec<u8>>, x_0_value: Bytes32) -> Result<KzgProof> {
        // Asserting that there is only one blob in the whole Vec<Vec<u8>> array for now.
        // Later we will add the support for multiple blob in single blob_data vec.
        assert_eq!(blob_data.len(), 1);

        let fixed_size_blob: [u8; BYTES_PER_BLOB] = blob_data[0].as_slice().try_into()?;

        let blob = Blob::new(fixed_size_blob);
        let commitment = KzgCommitment::blob_to_kzg_commitment(&blob, &KZG_SETTINGS)?;
        let (kzg_proof, y_0_value) = KzgProof::compute_kzg_proof(&blob, &x_0_value, &KZG_SETTINGS)?;

        // Verifying the proof for double check
        let eval = KzgProof::verify_kzg_proof(
            &commitment.to_bytes(),
            &x_0_value,
            &y_0_value,
            &kzg_proof.to_bytes(),
            &KZG_SETTINGS,
        )?;

        if !eval {
            Err(eyre!("ERROR : Assertion failed, not able to verify the proof."))
        } else {
            Ok(kzg_proof)
        }
    }
}

#[automock]
#[async_trait]
impl SettlementClient for EthereumSettlementClient {
    /// Should register the proof on the base layer and return an external id
    /// which can be used to track the status.
    #[allow(unused)]
    async fn register_proof(&self, proof: [u8; 32]) -> Result<String> {
        todo!("register_proof is not implemented yet")
    }

    /// Should be used to update state on core contract when DA is done in calldata
    async fn update_state_calldata(
        &self,
        program_output: Vec<[u8; 32]>,
        onchain_data_hash: [u8; 32],
        onchain_data_size: usize,
    ) -> Result<String> {
        let program_output: Vec<U256> = slice_slice_u8_to_vec_u256(program_output.as_slice());
        let onchain_data_hash: U256 = slice_u8_to_u256(&onchain_data_hash);
        let onchain_data_size: U256 = onchain_data_size.try_into()?;
        let tx_receipt =
            self.core_contract_client.update_state(program_output, onchain_data_hash, onchain_data_size).await?;
        Ok(format!("0x{:x}", tx_receipt.transaction_hash))
    }

    /// Should be used to update state on core contract when DA is in blobs/alt DA
    async fn update_state_blobs(&self, program_output: Vec<[u8; 32]>, kzg_proof: [u8; 48]) -> Result<String> {
        let program_output: Vec<U256> = slice_slice_u8_to_vec_u256(&program_output);
        let tx_receipt = self.core_contract_client.update_state_kzg(program_output, kzg_proof).await?;
        Ok(format!("0x{:x}", tx_receipt.transaction_hash))
    }

    async fn update_state_with_blobs(&self, program_output: Vec<[u8; 32]>, state_diff: Vec<Vec<u8>>) -> Result<String> {
        let trusted_setup = KzgSettings::load_trusted_setup_file(Path::new("./trusted_setup.txt"))
            .expect("issue while loading the trusted setup");
        let (sidecar_blobs, sidecar_commitments, sidecar_proofs) = prepare_sidecar(&state_diff, &trusted_setup).await?;
        let sidecar = BlobTransactionSidecar::new(sidecar_blobs, sidecar_commitments, sidecar_proofs);

        let eip1559_est = self.provider.estimate_eip1559_fees(None).await?;
        let chain_id: u64 = self.provider.get_chain_id().await?.to_string().parse()?;

        let max_fee_per_blob_gas: u128 = self.provider.get_blob_base_fee().await?.to_string().parse()?;
        let max_priority_fee_per_gas: u128 = self.provider.get_max_priority_fee_per_gas().await?.to_string().parse()?;

        let nonce = self.provider.get_transaction_count(self.wallet_address).await?.to_string().parse()?;

        // x_0_value : program_output[6]
        let kzg_proof = Self::build_proof(
            state_diff,
            Bytes32::from_bytes(program_output[6].as_slice()).expect("Not able to get x_0 point params."),
        )
        .await
        .expect("Unable to build KZG proof for given params.")
        .to_owned();

        let tx = TxEip4844 {
            chain_id,
            nonce,
            gas_limit: 30_000_000,
            max_fee_per_gas: eip1559_est.max_fee_per_gas.to_string().parse()?,
            max_priority_fee_per_gas,
            to: self.core_contract_client.contract_address(),
            value: U256::from(0),
            access_list: AccessList(vec![]),
            blob_versioned_hashes: sidecar.versioned_hashes().collect(),
            max_fee_per_blob_gas,
            input: get_txn_input_bytes(program_output, kzg_proof),
        };
        let tx_sidecar = TxEip4844WithSidecar { tx: tx.clone(), sidecar: sidecar.clone() };
        let mut variant = TxEip4844Variant::from(tx_sidecar);

        // Sign and submit
        let signature = self.wallet.default_signer().sign_transaction(&mut variant).await?;
        let tx_signed = variant.into_signed(signature);
        let tx_envelope: TxEnvelope = tx_signed.into();
        let encoded = tx_envelope.encoded_2718();

        let pending_tx = self.provider.send_raw_transaction(&encoded).await?;

        Ok(pending_tx.tx_hash().to_string())
    }

    /// Should verify the inclusion of a tx in the settlement layer
    async fn verify_tx_inclusion(&self, tx_hash: &str) -> Result<SettlementVerificationStatus> {
        let tx_hash = B256::from_str(tx_hash)?;
        let maybe_tx_status: Option<TransactionReceipt> = self.provider.get_transaction_receipt(tx_hash).await?;
        match maybe_tx_status {
            Some(tx_status) => {
                if tx_status.status() {
                    Ok(SettlementVerificationStatus::Verified)
                } else {
                    Ok(SettlementVerificationStatus::Pending)
                }
            }
            None => Ok(SettlementVerificationStatus::Rejected(format!("Could not find status of tx: {}", tx_hash))),
        }
    }

    /// Wait for a pending tx to achieve finality
    async fn wait_for_tx_finality(&self, tx_hash: &str) -> Result<()> {
        let tx_hash = B256::from_str(tx_hash)?;
        self.provider.watch_pending_transaction(PendingTransactionConfig::new(tx_hash)).await?;
        Ok(())
    }

    /// Get the last block settled through the core contract
    async fn get_last_settled_block(&self) -> Result<u64> {
        let block_number = self.core_contract_client.state_block_number().await?;
        Ok(block_number.try_into()?)
    }
}

/// To prepare the sidecar for EIP 4844 transaction
async fn prepare_sidecar(
    state_diff: &[Vec<u8>],
    trusted_setup: &KzgSettings,
) -> Result<(Vec<FixedBytes<131072>>, Vec<FixedBytes<48>>, Vec<FixedBytes<48>>)> {
    let mut sidecar_blobs = vec![];
    let mut sidecar_commitments = vec![];
    let mut sidecar_proofs = vec![];

    for blob_data in state_diff {
        let fixed_size_blob: [u8; BYTES_PER_BLOB] = blob_data.as_slice().try_into()?;

        let blob = Blob::new(fixed_size_blob);

        let commitment = KzgCommitment::blob_to_kzg_commitment(&blob, trusted_setup)?;
        let proof = KzgProof::compute_blob_kzg_proof(&blob, &commitment.to_bytes(), trusted_setup)?;

        sidecar_blobs.push(FixedBytes::new(fixed_size_blob));
        sidecar_commitments.push(FixedBytes::new(commitment.to_bytes().into_inner()));
        sidecar_proofs.push(FixedBytes::new(proof.to_bytes().into_inner()));
    }

    Ok((sidecar_blobs, sidecar_commitments, sidecar_proofs))
}

/// Function to construct the transaction for updating the state in core contract.
fn get_txn_input_bytes(program_output: Vec<[u8; 32]>, kzg_proof: [u8; 48]) -> Bytes {
    let program_output_hex_string = vec_u8_32_to_hex_string(program_output);
    let kzg_proof_hex_string = u8_48_to_hex_string(kzg_proof);
    // cast keccak "updateStateKzgDA(uint256[] calldata programOutput, bytes calldata kzgProof)" | cut -b 1-10
    let function_selector = "0x1a790556";

    Bytes::from(program_output_hex_string + &kzg_proof_hex_string + function_selector)
}

fn vec_u8_32_to_hex_string(data: Vec<[u8; 32]>) -> String {
    data.into_iter().fold(String::new(), |mut output, arr| {
        // Convert the array to a hex string
        let hex = arr.iter().fold(String::new(), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        });

        // Ensure the hex string is exactly 64 characters (32 bytes)
        let _ = write!(output, "{hex:0>64}");
        output
    })
}

fn u8_48_to_hex_string(data: [u8; 48]) -> String {
    // Split the array into two parts
    let (first_32, last_16) = data.split_at(32);

    // Convert and pad each part
    let first_hex = to_padded_hex(first_32);
    let second_hex = to_padded_hex(last_16);

    // Concatenate the two hex strings
    first_hex + &second_hex
}

// Function to convert a slice of u8 to a padded hex string
fn to_padded_hex(slice: &[u8]) -> String {
    let hex = slice.iter().fold(String::new(), |mut output, byte| {
        let _ = write!(output, "{byte:02x}");
        output
    });
    format!("{:0<64}", hex)
}

#[rstest]
fn test_data_conversion() {
    let data: [u8; 48] = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
        31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
    ];

    let result = u8_48_to_hex_string(data);

    assert_eq!(result, "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f3000000000000000000000000000000000");

    let mut data_2: [u8; 32] = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
        31, 32,
    ];
    let mut data_vec: Vec<[u8; 32]> = Vec::new();
    data_vec.push(data_2);
    data_2.reverse();
    data_vec.push(data_2);

    let data_3: [u8; 32] = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
        0, 0,
    ];
    data_vec.push(data_3);

    let result_2 = vec_u8_32_to_hex_string(data_vec);

    assert_eq!(result_2, "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20201f1e1d1c1b1a191817161514131211100f0e0d0c0b0a0908070605040302010102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e0000");
}

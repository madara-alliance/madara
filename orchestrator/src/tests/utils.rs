use crate::types::batch::{Batch, ClassDeclaration, ContractUpdate, DataJson, StorageUpdate};
use chrono::{SubsecRound, Utc};
use rstest::fixture;
use uuid::Uuid;

use crate::tests::config::{ConfigType, MockType, TestConfigBuilder, TestConfigBuilderReturns};
use crate::tests::jobs::snos_job::SNOS_PATHFINDER_RPC_URL_ENV;
use crate::types::constant::{
    BLOB_DATA_FILE_NAME, CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME,
};
use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata, SnosMetadata,
    StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use color_eyre::Result;
use num_bigint::BigUint;
use num_traits::Zero;
use starknet_core::types::StateUpdate;
use std::fs::File;
use std::hash::Hash;
use std::io::Read;
use tracing::error;
use tracing::log::warn;
use url::Url;

pub fn build_job_item(job_type: JobType, job_status: JobStatus, internal_id: u64) -> JobItem {
    let metadata = match job_type {
        JobType::StateTransition => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
                blocks_to_settle: vec![internal_id],
                snos_output_paths: vec![format!("{}/{}", internal_id, SNOS_OUTPUT_FILE_NAME)],
                program_output_paths: vec![format!("{}/{}", internal_id, PROGRAM_OUTPUT_FILE_NAME)],
                blob_data_paths: vec![format!("{}/{}", internal_id, BLOB_DATA_FILE_NAME)],
                last_failed_block_no: None,
                tx_hashes: Vec::new(),
            }),
        },
        JobType::SnosRun => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Snos(SnosMetadata {
                block_number: internal_id,
                cairo_pie_path: Some(format!("{}/{}", internal_id, CAIRO_PIE_FILE_NAME)),
                snos_output_path: Some(format!("{}/{}", internal_id, SNOS_OUTPUT_FILE_NAME)),
                program_output_path: Some(format!("{}/{}", internal_id, PROGRAM_OUTPUT_FILE_NAME)),
                ..Default::default()
            }),
        },
        JobType::ProofCreation => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Proving(ProvingMetadata {
                block_number: internal_id,
                input_path: Some(ProvingInputType::CairoPie(format!("{}/{}", internal_id, CAIRO_PIE_FILE_NAME))),
                ..Default::default()
            }),
        },
        JobType::DataSubmission => JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Da(DaMetadata {
                block_number: internal_id,
                blob_data_path: Some(format!("{}/{}", internal_id, BLOB_DATA_FILE_NAME)),
                tx_hash: None,
            }),
        },
        _ => panic!("Invalid job type"),
    };

    JobItem {
        id: Uuid::new_v4(),
        internal_id: internal_id.to_string(),
        job_type,
        status: job_status,
        external_id: ExternalId::Number(0),
        metadata,
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    }
}

#[fixture]
pub fn build_batch(
    #[default(1)] index: u64,
    #[default(100)] start_block: u64,
    #[default(200)] end_block: u64,
) -> Batch {
    Batch {
        id: Uuid::new_v4(),
        index,
        num_blocks: end_block - start_block + 1,
        start_block,
        end_block,
        is_batch_ready: false,
        squashed_state_updates_path: String::from("path/to/file.json"),
        blob_path: String::from("path/to/file.json"),
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    }
}

#[fixture]
pub async fn build_test_config_with_real_provider() -> Result<TestConfigBuilderReturns> {
    let pathfinder_url: Url = match std::env::var(SNOS_PATHFINDER_RPC_URL_ENV) {
        Ok(url) => url.parse()?,
        Err(_) => {
            return Err(color_eyre::eyre::eyre!("{} environment variable is not set", SNOS_PATHFINDER_RPC_URL_ENV));
        }
    };

    Ok(TestConfigBuilder::new().configure_rpc_url(ConfigType::Mock(MockType::RpcUrl(pathfinder_url))).build().await)
}

pub fn read_state_updates_vec_from_file(file_path: &str) -> Result<Vec<StateUpdate>> {
    Ok(serde_json::from_str(&read_file_to_string(file_path)?)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse state update vector file: {}", e))?)
}

pub fn read_state_update_from_file(file_path: &str) -> Result<StateUpdate> {
    Ok(serde_json::from_str(&read_file_to_string(file_path)?)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse state update file: {}", e))?)
}

pub fn read_file_to_string(file_path: &str) -> Result<String> {
    let mut string = String::new();
    let mut file = File::open(file_path)?;
    file.read_to_string(&mut string)?;
    Ok(string)
}

pub fn read_data_json_from_file(file_path: &str) -> Result<DataJson> {
    Ok(parse_json_to_data_json(&read_file_to_string(file_path)?)?)
}

pub fn parse_json_to_data_json(json_str: &str) -> Result<DataJson> {
    // First try direct deserialization
    let result = serde_json::from_str::<DataJson>(json_str);

    match result {
        Ok(data_json) => Ok(data_json),
        Err(e) => {
            error!("Warning: Standard deserialization failed: {}", e);

            // If direct deserialization fails, try parsing as a Value first
            let json_value: serde_json::Value = serde_json::from_str(json_str)?;

            let mut state_updates = Vec::new();
            let mut class_declarations = Vec::new();

            // Parse state updates
            if let Some(updates) = json_value.get("state_update").and_then(|u| u.as_array()) {
                for update in updates {
                    if let Some(update_obj) = update.as_object() {
                        // Parse address
                        let address = if let Some(addr) = update_obj.get("address") {
                            if let Some(addr_str) = addr.as_str() {
                                addr_str.parse::<BigUint>().unwrap_or(BigUint::zero())
                            } else if let Some(addr_num) = addr.as_u64() {
                                BigUint::from(addr_num)
                            } else {
                                BigUint::zero()
                            }
                        } else {
                            BigUint::zero()
                        };

                        // Parse nonce
                        let nonce = update_obj.get("nonce").and_then(|n| n.as_u64()).unwrap_or(0);

                        // Parse number of storage updates
                        let number_of_storage_updates =
                            update_obj.get("number_of_storage_updates").and_then(|n| n.as_u64()).unwrap_or(0);

                        // Parse class hash if present
                        let new_class_hash = update_obj.get("new_class_hash").and_then(|h| {
                            if h.is_null() {
                                None
                            } else if let Some(hash_str) = h.as_str() {
                                Some(hash_str.parse::<BigUint>().unwrap_or(BigUint::zero()))
                            } else if let Some(hash_num) = h.as_u64() {
                                Some(BigUint::from(hash_num))
                            } else {
                                None
                            }
                        });

                        // Parse storage updates
                        let mut storage_updates = Vec::new();
                        if let Some(storage_array) = update_obj.get("storage_updates").and_then(|s| s.as_array()) {
                            for storage in storage_array {
                                if let Some(storage_obj) = storage.as_object() {
                                    // Parse key
                                    let key = if let Some(key_val) = storage_obj.get("key") {
                                        if let Some(key_str) = key_val.as_str() {
                                            key_str.parse::<BigUint>().unwrap_or(BigUint::zero())
                                        } else if let Some(key_num) = key_val.as_u64() {
                                            BigUint::from(key_num)
                                        } else {
                                            BigUint::zero()
                                        }
                                    } else {
                                        BigUint::zero()
                                    };

                                    // Parse value
                                    let value = if let Some(value_val) = storage_obj.get("value") {
                                        if let Some(value_str) = value_val.as_str() {
                                            value_str.parse::<BigUint>().unwrap_or(BigUint::zero())
                                        } else if let Some(value_num) = value_val.as_u64() {
                                            BigUint::from(value_num)
                                        } else {
                                            BigUint::zero()
                                        }
                                    } else {
                                        BigUint::zero()
                                    };

                                    storage_updates.push(StorageUpdate { key, value });
                                }
                            }
                        }

                        state_updates.push(ContractUpdate {
                            address,
                            nonce,
                            number_of_storage_updates,
                            new_class_hash,
                            storage_updates,
                        });
                    }
                }
            }

            // Parse class declarations
            if let Some(declarations) = json_value.get("class_declaration").and_then(|d| d.as_array()) {
                for decl in declarations {
                    if let Some(decl_obj) = decl.as_object() {
                        // Parse class hash
                        let class_hash = if let Some(hash) = decl_obj.get("class_hash") {
                            if let Some(hash_str) = hash.as_str() {
                                hash_str.parse::<BigUint>().unwrap_or(BigUint::zero())
                            } else if let Some(hash_num) = hash.as_u64() {
                                BigUint::from(hash_num)
                            } else {
                                BigUint::zero()
                            }
                        } else {
                            BigUint::zero()
                        };

                        // Parse compiled class hash
                        let compiled_class_hash = if let Some(hash) = decl_obj.get("compiled_class_hash") {
                            if let Some(hash_str) = hash.as_str() {
                                hash_str.parse::<BigUint>().unwrap_or(BigUint::zero())
                            } else if let Some(hash_num) = hash.as_u64() {
                                BigUint::from(hash_num)
                            } else {
                                BigUint::zero()
                            }
                        } else {
                            BigUint::zero()
                        };

                        class_declarations.push(ClassDeclaration { class_hash, compiled_class_hash });
                    }
                }
            }

            // Get sizes
            let state_update_size =
                json_value.get("state_update_size").and_then(|s| s.as_u64()).unwrap_or(state_updates.len() as u64);

            let class_declaration_size = json_value
                .get("class_declaration_size")
                .and_then(|s| s.as_u64())
                .unwrap_or(class_declarations.len() as u64);

            Ok(DataJson {
                state_update_size,
                state_update: state_updates,
                class_declaration_size,
                class_declaration: class_declarations,
            })
        }
    }
}

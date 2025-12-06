//! Startup information logging
//!
//! This module provides comprehensive logging of orchestrator configuration at startup.
//! It displays all critical configuration information in a clear, structured format.

use crate::core::config::Config;
use crate::types::params::da::DAConfig;
use crate::types::params::prover::ProverConfig;
use crate::types::params::settlement::SettlementConfig;
use tracing::info;

/// Log comprehensive startup information
pub fn log_startup_info(config: &Config) {
    info!("═══════════════════════════════════════════════════════════════════");
    info!("                  Madara Orchestrator Starting                     ");
    info!("═══════════════════════════════════════════════════════════════════");

    log_deployment_info(config);
    log_madara_config(config);
    log_settlement_config(config);
    log_da_config(config);
    log_prover_config(config);
    log_snos_config(config);
    log_batching_config(config);
    log_service_config(config);
    log_job_policies(config);
    log_server_config(config);
    log_operational_config(config);

    info!("═══════════════════════════════════════════════════════════════════");
    info!("Orchestrator initialization complete. Ready to process blocks.");
    info!("═══════════════════════════════════════════════════════════════════");
}

fn log_deployment_info(config: &Config) {
    info!("┌─ Deployment Configuration");
    info!("│  Layer: {:?}", config.layer());
    info!("└─");
}

fn log_madara_config(config: &Config) {
    info!("┌─ Madara Configuration");
    info!("│  RPC URL: {}", config.params.madara_rpc_url);
    info!("│  Feeder Gateway URL: {}", config.params.madara_feeder_gateway_url);
    info!("│  Version: {}", config.params.madara_version);
    info!("└─");
}

fn log_settlement_config(_config: &Config) {
    info!("┌─ Settlement Layer Configuration");
    info!("│  Settlement client initialized");
    info!("│  (Run with RUST_LOG=debug for detailed settlement configuration)");
    info!("└─");
}

fn log_da_config(_config: &Config) {
    info!("┌─ Data Availability Configuration");
    info!("│  DA client initialized");
    info!("│  (Run with RUST_LOG=debug for detailed DA configuration)");
    info!("└─");
}

fn log_prover_config(config: &Config) {
    info!("┌─ Prover Configuration");
    info!("│  Prover client initialized");
    info!("│  Prover Layout: {}", config.prover_layout_name());
    info!("│  (Run with RUST_LOG=debug for detailed prover configuration)");
    info!("└─");
}

fn log_snos_config(config: &Config) {
    info!("┌─ SNOS Configuration");
    info!("│  RPC URL: {}", config.snos_config().rpc_for_snos);
    info!("│  Layout: {}", config.snos_layout_name());
    info!("│  Full Output: {}", config.snos_config().snos_full_output);
    info!("│  STRK Fee Token Address: {}", config.snos_config().strk_fee_token_address);
    info!("│  ETH Fee Token Address: {}", config.snos_config().eth_fee_token_address);
    if config.snos_config().versioned_constants.is_some() {
        info!("│  Using Custom Versioned Constants: Yes");
    }
    info!("└─");
}

fn log_batching_config(config: &Config) {
    info!("┌─ Batching Configuration");
    info!("│  Max Batch Time: {}s", config.params.batching_config.max_batch_time_seconds);
    info!("│  Max Batch Size: {} blocks", config.params.batching_config.max_batch_size);
    info!("│  Max Number of Blobs: {}", config.params.batching_config.max_num_blobs);

    if let Some(max_blocks_per_batch) = config.params.batching_config.max_blocks_per_snos_batch {
        info!("│  Max Blocks Per SNOS Batch: {}", max_blocks_per_batch);
    }

    // Log bouncer weights
    let bouncer = config.bouncer_weights_limit();
    info!("│  Bouncer Weights:");
    info!("│    L1 Gas: {}", bouncer.l1_gas);
    info!("│    Message Segment Length: {}", bouncer.message_segment_length);
    info!("│    Events: {}", bouncer.n_events);
    info!("│    State Diff Size: {}", bouncer.state_diff_size);
    info!("│    Transactions: {}", bouncer.n_txs);
    info!("└─");
}

fn log_service_config(config: &Config) {
    info!("┌─ Service Configuration");
    info!("│  Min Block to Process: {}", config.service_config().min_block_to_process);

    if let Some(max_block) = config.service_config().max_block_to_process {
        info!("│  Max Block to Process: {}", max_block);
    }

    info!("│  Max Concurrent Created SNOS Jobs: {}", config.service_config().max_concurrent_created_snos_jobs);

    if let Some(max_snos) = config.service_config().max_concurrent_snos_jobs {
        info!("│  Max Concurrent SNOS Jobs: {}", max_snos);
    }

    if let Some(max_proving) = config.service_config().max_concurrent_proving_jobs {
        info!("│  Max Concurrent Proving Jobs: {}", max_proving);
    }

    info!("│  Job Processing Timeout: {}s", config.service_config().job_processing_timeout_seconds);
    info!("└─");
}

fn log_job_policies(config: &Config) {
    info!("┌─ Job Retry & Timeout Policies");

    let policies = &config.params.job_policies;

    info!("│  SNOS Execution:");
    info!("│    Process Attempts: {}", policies.snos_execution.max_process_attempts);
    info!("│    Verification Attempts: {}", policies.snos_execution.max_verification_attempts);
    info!("│    Polling Delay: {}s", policies.snos_execution.verification_polling_delay_seconds);

    info!("│  Proving:");
    info!("│    Process Attempts: {}", policies.proving.max_process_attempts);
    info!("│    Verification Attempts: {}", policies.proving.max_verification_attempts);
    info!("│    Polling Delay: {}s", policies.proving.verification_polling_delay_seconds);

    info!("│  DA Submission:");
    info!("│    Process Attempts: {}", policies.da_submission.max_process_attempts);
    info!("│    Verification Attempts: {}", policies.da_submission.max_verification_attempts);
    info!("│    Polling Delay: {}s", policies.da_submission.verification_polling_delay_seconds);

    info!("│  State Update:");
    info!("│    Process Attempts: {}", policies.state_update.max_process_attempts);
    info!("│    Verification Attempts: {}", policies.state_update.max_verification_attempts);
    info!("│    Polling Delay: {}s", policies.state_update.verification_polling_delay_seconds);

    info!("│  Aggregator:");
    info!("│    Process Attempts: {}", policies.aggregator.max_process_attempts);
    info!("│    Verification Attempts: {}", policies.aggregator.max_verification_attempts);
    info!("│    Polling Delay: {}s", policies.aggregator.verification_polling_delay_seconds);

    info!("│  Proof Registration:");
    info!("│    Process Attempts: {}", policies.proof_registration.max_process_attempts);
    info!("│    Verification Attempts: {}", policies.proof_registration.max_verification_attempts);
    info!("│    Polling Delay: {}s", policies.proof_registration.verification_polling_delay_seconds);

    info!("└─");
}

fn log_server_config(config: &Config) {
    info!("┌─ Server Configuration");
    info!("│  Host: {}", config.server_config().host);
    info!("│  Port: {}", config.server_config().port);
    info!("└─");
}

fn log_operational_config(config: &Config) {
    info!("┌─ Operational Settings");
    info!("│  Store Audit Artifacts: {}", config.params.store_audit_artifacts);
    info!("└─");
}

/// Log detailed configuration at debug level
/// This provides additional information for troubleshooting
pub fn log_detailed_config(settlement_config: &SettlementConfig, da_config: &DAConfig, prover_config: &ProverConfig) {
    use tracing::debug;

    debug!("┌─ Settlement Layer Details");
    match settlement_config {
        SettlementConfig::Ethereum(eth_config) => {
            debug!("│  Type: Ethereum");
            debug!("│  RPC URL: {}", eth_config.ethereum_rpc_url);
            debug!("│  Core Contract: {}", eth_config.l1_core_contract_address);
            debug!("│  Operator Address: {}", eth_config.starknet_operator_address);
            debug!("│  Finality Retry Wait: {}s", eth_config.ethereum_finality_retry_wait_in_secs);
            debug!("│  Max Gas Price Multiplier: {}", eth_config.max_gas_price_mul_factor);
            debug!("│  Disable PeerDAS: {}", eth_config.disable_peerdas);
        }
        SettlementConfig::Starknet(sn_config) => {
            debug!("│  Type: Starknet");
            debug!("│  RPC URL: {}", sn_config.starknet_rpc_url);
            debug!("│  Account Address: {}", sn_config.starknet_account_address);
            debug!("│  Cairo Core Contract: {}", sn_config.starknet_cairo_core_contract_address);
            debug!("│  Finality Retry Wait: {}s", sn_config.starknet_finality_retry_wait_in_secs);
        }
    }
    debug!("└─");

    debug!("┌─ Data Availability Details");
    match da_config {
        DAConfig::Ethereum(eth_da) => {
            debug!("│  Type: Ethereum");
            debug!("│  RPC URL: {}", eth_da.ethereum_da_rpc_url);
        }
        DAConfig::Starknet(sn_da) => {
            debug!("│  Type: Starknet");
            debug!("│  RPC URL: {}", sn_da.starknet_da_rpc_url);
        }
    }
    debug!("└─");

    debug!("┌─ Prover Details");
    match prover_config {
        ProverConfig::Sharp(sharp) => {
            debug!("│  Type: SHARP");
            debug!("│  Service URL: {}", sharp.sharp_url);
            debug!("│  Customer ID: {}", sharp.sharp_customer_id);
            debug!("│  Settlement Layer: {}", sharp.sharp_settlement_layer);
            debug!("│  Verifier Contract: {}", sharp.gps_verifier_contract_address);
        }
        ProverConfig::Atlantic(atlantic) => {
            debug!("│  Type: Atlantic");
            debug!("│  Service URL: {}", atlantic.atlantic_service_url);
            debug!("│  Network: {}", atlantic.atlantic_network);
            debug!("│  Prover Type: {}", atlantic.atlantic_prover_type);
            debug!("│  Verifier Contract: {}", atlantic.atlantic_verifier_contract_address);
            debug!("│  Settlement Layer: {}", atlantic.atlantic_settlement_layer);
            debug!("│  Cairo VM: {:?}", atlantic.atlantic_cairo_vm);
            if let Some(verifier_hash) = &atlantic.cairo_verifier_program_hash {
                debug!("│  Cairo Verifier Program Hash: {}", verifier_hash);
            }
        }
    }
    debug!("└─");
}

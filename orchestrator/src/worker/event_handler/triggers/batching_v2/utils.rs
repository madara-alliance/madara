use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::utils::rest_client::RestClient;
use blockifier::bouncer::BouncerWeights;
use color_eyre::eyre::eyre;
use std::sync::Arc;
use tracing::debug;

pub async fn get_block_builtin_weights(
    block_number: u64,
    fgw: &Arc<RestClient>,
    empty_block_proving_gas: u64,
) -> Result<BouncerWeights, JobError> {
    debug!(
        block_number = %block_number,
        "Requesting block bouncer weights via REST"
    );

    // Use the RestClient with query parameters
    let response = fgw
        .get(&format!("/feeder_gateway/get_block_bouncer_weights?blockNumber={}", block_number))
        .await
        .map_err(|e| JobError::Other(OtherError(eyre!("Failed to send REST request: {}", e))))?;

    // Check for HTTP errors
    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_else(|_| "Unable to read error response".to_string());
        return Err(JobError::Other(OtherError(eyre!("REST request failed with status {}: {}", status, error_text))));
    }

    // Parse the response
    let mut bouncer_weights: BouncerWeights = response
        .json()
        .await
        .map_err(|e| JobError::Other(OtherError(eyre!("Failed to parse REST response: {}", e))))?;

    // If proving_gas is zero (empty block), use the configured default value.
    // Every block has some proving cost regardless of transactions.
    if bouncer_weights.proving_gas.0 == 0 {
        debug!(
            block_number = %block_number,
            default_proving_gas = %empty_block_proving_gas,
            "Block has zero proving_gas (empty block), using default value"
        );
        bouncer_weights.proving_gas = starknet_api::execution_resources::GasAmount(empty_block_proving_gas);
    }

    Ok(bouncer_weights)
}

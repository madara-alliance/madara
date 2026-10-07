use crate::cli::SnosRunnerCmd;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use generate_pie::types::{ChainConfig, OsHintsConfiguration, PieGenerationInput, PieGenerationTiming};
use generate_pie::{execute_prepared_pie, prepare_pie, prepare_pie_from_witness, RpcWitness};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;

#[derive(Debug)]
struct RunnerState {
    config: SnosRunnerCmd,
    client: reqwest::Client,
    run_slot: Semaphore,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RunMode {
    Rpc,
    Witness,
}

#[derive(Debug, Deserialize)]
struct RunRequest {
    blocks: Vec<u64>,
    mode: RunMode,
    #[serde(default = "default_include_zip")]
    include_zip: bool,
}

fn default_include_zip() -> bool {
    true
}

#[derive(Debug, Serialize)]
struct RunResponse {
    success: bool,
    mode: &'static str,
    blocks: Vec<u64>,
    total_ms: u64,
    witness_fetch_ms: u64,
    preparation_ms: u64,
    os_execution_ms: u64,
    pie_zip_ms: u64,
    pie_zip_bytes: Option<usize>,
    rpc_wait_ms: u64,
    local_processing_ms: u64,
    rpc_calls: HashMap<String, u64>,
    witness_responses: Option<usize>,
    cairo_steps: usize,
}

pub async fn run(config: SnosRunnerCmd) -> anyhow::Result<()> {
    let address = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    let state = Arc::new(RunnerState { config, client: reqwest::Client::new(), run_slot: Semaphore::new(1) });
    let app = Router::new().route("/health", get(|| async { "OK" })).route("/run", post(run_snos)).with_state(state);
    tracing::info!(address, "Lightweight SNOS A/B runner listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn run_snos(
    State(state): State<Arc<RunnerState>>,
    Json(request): Json<RunRequest>,
) -> Result<Json<RunResponse>, (StatusCode, String)> {
    if request.blocks.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "blocks must not be empty".to_owned()));
    }
    if !request.blocks.windows(2).all(|window| window[0] < window[1]) {
        return Err((StatusCode::BAD_REQUEST, "blocks must be strictly increasing".to_owned()));
    }
    let _run_slot = state
        .run_slot
        .try_acquire()
        .map_err(|_| (StatusCode::TOO_MANY_REQUESTS, "a SNOS run is already active".to_owned()))?;

    run_snos_inner(&state, request)
        .await
        .map(Json)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}")))
}

async fn run_snos_inner(state: &RunnerState, request: RunRequest) -> anyhow::Result<RunResponse> {
    let started_at = Instant::now();
    let input = pie_input(&state.config, request.blocks.clone())?;
    let (witness, witness_responses, witness_fetch_ms, mode) = match request.mode {
        RunMode::Rpc => (None, None, 0, "rpc"),
        RunMode::Witness => {
            let witness_started_at = Instant::now();
            let mut witnesses = Vec::with_capacity(request.blocks.len());
            for block_number in &request.blocks {
                let url = state.config.witness_url.join("feeder_gateway/get_block_witness")?;
                let witness = state
                    .client
                    .get(url)
                    .query(&[("blockNumber", block_number)])
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<RpcWitness>()
                    .await?;
                witnesses.push(witness);
            }
            let witness = RpcWitness::merge(witnesses)?;
            let response_count = witness.response_count();
            (Some(witness), Some(response_count), elapsed_ms(witness_started_at), "witness")
        }
    };
    let preparation_started_at = Instant::now();
    let prepared = match witness {
        Some(witness) => prepare_pie_from_witness(input, witness).await?,
        None => prepare_pie(input).await?,
    };
    let preparation_ms = elapsed_ms(preparation_started_at);

    let execution_started_at = Instant::now();
    let result = tokio::task::spawn_blocking(move || execute_prepared_pie(prepared)).await??;
    let os_execution_ms = elapsed_ms(execution_started_at);
    let timing = result.timing;
    let cairo_steps = result.output.cairo_pie.execution_resources.n_steps;

    let zip_started_at = Instant::now();
    let pie_zip_bytes = if request.include_zip {
        let pie = result.output.cairo_pie;
        Some(
            tokio::task::spawn_blocking(move || crate::worker::utils::pie::cairo_pie_to_zip_bytes_blocking(pie))
                .await?
                .map_err(|error| anyhow::anyhow!("{error:#}"))?
                .len(),
        )
    } else {
        None
    };
    let pie_zip_ms = if request.include_zip { elapsed_ms(zip_started_at) } else { 0 };

    let PieGenerationTiming { rpc_wait_time_ms, execution_time_ms, rpc_calls_by_method, .. } = timing;
    Ok(RunResponse {
        success: true,
        mode,
        blocks: request.blocks,
        total_ms: elapsed_ms(started_at),
        witness_fetch_ms,
        preparation_ms,
        os_execution_ms,
        pie_zip_ms,
        pie_zip_bytes,
        rpc_wait_ms: rpc_wait_time_ms,
        local_processing_ms: execution_time_ms,
        rpc_calls: rpc_calls_by_method,
        witness_responses,
        cairo_steps,
    })
}

fn pie_input(config: &SnosRunnerCmd, blocks: Vec<u64>) -> anyhow::Result<PieGenerationInput> {
    Ok(PieGenerationInput {
        rpc_url: config.rpc_url.as_str().trim_end_matches('/').to_owned(),
        blocks,
        layout: generate_pie::parse_layout("all_cairo")?,
        chain_config: ChainConfig::new(
            &config.chain_id,
            &config.strk_fee_token_address,
            &config.eth_fee_token_address,
            config.is_l3,
        ),
        os_hints_config: OsHintsConfiguration::default_with_is_l3(config.is_l3),
        output_path: None,
        versioned_constants: config.versioned_constants.clone(),
        public_keys: None,
    })
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

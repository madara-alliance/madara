//! Contains the code required to fetch data from the feeder efficiently.

use std::time::Duration;
use mp_commitments::StateCommitment;
use reqwest::Url;
use serde::Deserialize;
use sp_core::H256;
use starknet_api::block::{BlockNumber, BlockHash};
use starknet_gateway::sequencer::models::BlockId;
use starknet_gateway::SequencerGatewayProvider;
use anyhow::Result;
use tokio::sync::mpsc::{Sender, self};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use std::sync::{Mutex, Arc};
use lazy_static::lazy_static;

use crate::state_updates::StarknetStateUpdate;
use crate::CommandSink;

/// Contains the Starknet verified state on L2
#[derive(Debug, Clone, Deserialize)]
pub struct L2StateUpdate {
    pub global_root: StateCommitment,
    pub block_number: BlockNumber,
    pub block_hash: BlockHash,
}

lazy_static! {
    /// Shared latest L2 state update verified on L2
    pub static ref STARKNET_STATE_UPDATE: Arc<Mutex<L2StateUpdate>> = Arc::new(Mutex::new(L2StateUpdate {
        global_root: StateCommitment::default(),
        block_number: BlockNumber::default(),
        block_hash: BlockHash::default(),
    }));
}

/// The configuration of the worker responsible for fetching new blocks and state updates from the
/// feeder.
pub struct FetchConfig {
    /// The URL of the sequencer gateway.
    pub gateway: Url,
    /// The URL of the feeder gateway.
    pub feeder_gateway: Url,
    /// The ID of the chain served by the sequencer gateway.
    pub chain_id: starknet_ff::FieldElement,
    /// The number of tasks spawned to fetch blocks and state updates.
    pub workers: u32,
    /// Whether to play a sound when a new block is fetched.
    pub sound: bool,
}

impl Default for FetchConfig {
    fn default() -> Self {
        FetchConfig {
            // Provide default values for each field of FetchConfig
            gateway: Url::parse("http://default-gateway-url.com").unwrap(),
            feeder_gateway: Url::parse("http://default-feeder-gateway-url.com").unwrap(),
            chain_id: starknet_ff::FieldElement::default(), // Adjust as necessary
            workers: 4,
            sound: false,
        }
    }
}

impl Clone for FetchConfig {
    fn clone(&self) -> Self {
        FetchConfig {
            gateway: self.gateway.clone(),
            feeder_gateway: self.feeder_gateway.clone(),
            chain_id: self.chain_id.clone(),
            workers: self.workers,
            sound: self.sound,
        }
    }
}

/// The configuration of the senders responsible for sending blocks and state updates from the
/// feeder.
pub struct SenderConfig {
    /// Sender for dispatching fetched blocks.
    pub block_sender: Sender<mp_block::Block>,
    /// Sender for dispatching fetched state updates.
    pub state_update_sender: Sender<StarknetStateUpdate>,
    /// The command sink used to notify the consensus engine that a new block should be created.
    pub command_sink: CommandSink,
}

/// Spawns workers to fetch blocks and state updates from the feeder.
pub async fn sync(mut sender_config: SenderConfig, config: FetchConfig, start_at: u64) {
    let SenderConfig { block_sender, state_update_sender, command_sink } = &mut sender_config;
    let client = SequencerGatewayProvider::new(config.gateway.clone(), config.feeder_gateway.clone(), config.chain_id);

    let mut current_block_number = start_at;
    let mut last_block_hash = None;
    let mut got_block = false;
    let mut got_state_update = false;
    loop {
        let (block, state_update) = match (got_block, got_state_update) {
            (false, false) => {
                let block = fetch_block(&client, block_sender, current_block_number);
                let state_update = fetch_state_update(&client, state_update_sender, current_block_number);
                tokio::join!(block, state_update)
            },
            (false, true) => {
                (fetch_block(&client, block_sender, current_block_number).await, Ok(()))
            },
            (true, false) => {
                (Ok(()), fetch_state_update(&client, state_update_sender, current_block_number).await)
            },
            (true, true) => unreachable!(),
        };
        
        got_block = got_block || block.is_ok();
        got_state_update = got_state_update || state_update.is_ok();
        
        match (block, state_update) {
            (Ok(()), Ok(())) => {
                match create_block(command_sink, &mut last_block_hash).await {
                    Ok(()) => {
                        current_block_number += 1;
                        got_block = false;
                        got_state_update = false;
                    }
                    Err(e) => {
                        eprintln!("Failed to create block: {}", e);
                        return;
                    }
                }
            }
            (Err(a), Ok(())) => {
                eprintln!("Failed to fetch block {}: {}", current_block_number, a);
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
            (_, Err(b)) => {
                eprintln!("Failed to fetch state update {}: {}", current_block_number, b);
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }
}

async fn fetch_block(
    client: &SequencerGatewayProvider,
    block_sender: &Sender<mp_block::Block>,
    block_number: u64,
) -> Result<(), String> {
    let block =
        client.get_block(BlockId::Number(block_number)).await.map_err(|e| format!("failed to get block: {e}"))?;

    block_sender.send(crate::convert::block(&block)).await.map_err(|e| format!("failed to dispatch block: {e}"))?;

    Ok(())
}

pub async fn fetch_genesis_block(
    config: FetchConfig
) -> Result<mp_block::Block, String> {
    let client = SequencerGatewayProvider::new(config.gateway.clone(), config.feeder_gateway.clone(), config.chain_id);
    let block =
        client.get_block(BlockId::Number(0)).await.map_err(|e| format!("failed to get block: {e}"))?;

    Ok(crate::convert::block(&block))
}

async fn fetch_state_update(
    client: &SequencerGatewayProvider,
    state_update_sender: &Sender<StarknetStateUpdate>,
    block_number: u64,
) -> Result<(), String> {
    let state_update = client
        .get_state_update(BlockId::Number(block_number))
        .await
        .map_err(|e| format!("failed to get state update: {e}"))?;

    // Now send state_update, which moves it
    state_update_sender
        .send(StarknetStateUpdate(state_update))
        .await
        .map_err(|e| format!("failed to dispatch state update: {e}"))?;

    Ok(())
}


/// Notifies the consensus engine that a new block should be created.
async fn create_block(cmds: &mut CommandSink, parent_hash: &mut Option<H256>) -> Result<(), String> {
    let (sender, receiver) = futures::channel::oneshot::channel();

    cmds.try_send(sc_consensus_manual_seal::rpc::EngineCommand::SealNewBlock {
        create_empty: true,
        finalize: true,
        parent_hash: None,
        sender: Some(sender),
    })
    .unwrap();

    let create_block_info = receiver
        .await
        .map_err(|err| format!("failed to seal block: {err}"))?
        .map_err(|err| format!("failed to seal block: {err}"))?;

    *parent_hash = Some(create_block_info.hash);
    Ok(())
}

/// Update the L2 state with the latest data
pub fn update_l2(state_update: L2StateUpdate) {
    {
        let last_state_update = STARKNET_STATE_UPDATE.clone();
        let mut new_state_update = last_state_update.lock().unwrap();
        *new_state_update = state_update.clone();
    }
}
#![allow(deprecated)]

use reqwest::Url;

mod convert;
mod l2;
mod utility;
mod l1;

pub use l2::BlockFetchConfig;

type CommandSink = futures_channel::mpsc::Sender<sc_consensus_manual_seal::rpc::EngineCommand<sp_core::H256>>;

pub async fn sync(command_sink: CommandSink, sender: tokio::sync::mpsc::Sender<mp_block::Block>, fetch_config: BlockFetchConfig, rpc_port: u16, l1_client_url: Url) {
    let first_block = utility::get_last_synced_block(rpc_port).await + 1;
    l2::sync(command_sink, sender, fetch_config, first_block).await;
    l1::sync(l1_client_url).await;
}

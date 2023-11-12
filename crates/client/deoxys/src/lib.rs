#![allow(deprecated)]

use reqwest::Url;
mod utility;
mod convert;
pub mod state_updates;
pub mod l2;
pub mod l1;
pub use l2::SenderConfig;
pub use l2::FetchConfig;
pub use l2::StarknetStateUpdate;

type CommandSink = futures::channel::mpsc::Sender<sc_consensus_manual_seal::rpc::EngineCommand<sp_core::H256>>;

pub async fn sync(
    command_sink: CommandSink,
    sender_config: SenderConfig,
    fetch_config: FetchConfig,
    rpc_port: u16, 
    l1_url: Url
) {
    let first_block = utility::get_last_synced_block(rpc_port).await + 1;    
    l2::sync(command_sink, sender_config, fetch_config, first_block).await;
    l1::sync(l1_url).await;
}

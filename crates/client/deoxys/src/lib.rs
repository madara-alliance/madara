#![allow(deprecated)]

use reqwest::Url;
pub mod utility;
pub mod convert;
pub mod state_updates;
pub mod l2;
pub mod l1;
pub use l2::SenderConfig;
pub use l2::FetchConfig;
use tokio::join;
#[cfg(feature = "m")]
mod m;

type CommandSink = futures::channel::mpsc::Sender<sc_consensus_manual_seal::rpc::EngineCommand<sp_core::H256>>;

pub async fn sync(
    sender_config: SenderConfig,
    fetch_config: FetchConfig,
    rpc_port: u16, 
    l1_url: Url
) {
    let first_block = utility::get_last_synced_block(rpc_port).await + 447770;   
 
    let _ = join!(
        l1::sync(l1_url, rpc_port),
        l2::sync(sender_config, fetch_config, first_block)
    );
}

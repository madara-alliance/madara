#![allow(deprecated)]
#![feature(let_chains)]

// use std::sync::Arc;
// use sp_runtime::traits::Block as BlockT;
// use reqwest::Url;

pub mod commitments;
pub mod l1;
pub mod l2;
pub mod types;
pub mod utils;

pub use l2::{FetchConfig, SenderConfig};
pub use types::state_updates;
pub use utils::{convert, m, utility};

type CommandSink = futures::channel::mpsc::Sender<sc_consensus_manual_seal::rpc::EngineCommand<sp_core::H256>>;

pub mod starknet_sync_worker {
    use super::*;
    use std::sync::Arc;
    use sp_runtime::traits::Block as BlockT;
    use reqwest::Url;

    pub async fn sync<B: BlockT>(
        fetch_config: FetchConfig,
        sender_config: SenderConfig,
        rpc_port: u16,
        l1_url: Url,
        backend: Arc<mc_db::Backend<B>>,
    ) {
        let first_block = utility::get_last_synced_block(rpc_port).await + 1;

        let _ = tokio::join!(
            l1::sync(l1_url.clone(), rpc_port),
            l2::sync(sender_config, fetch_config.clone(), first_block, rpc_port, backend.clone())
        );
    }
}
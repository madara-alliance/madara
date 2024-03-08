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
pub use utils::{convert, m, utility};

type CommandSink = futures::channel::mpsc::Sender<sc_consensus_manual_seal::rpc::EngineCommand<sp_core::H256>>;

pub mod starknet_sync_worker {
    use std::sync::Arc;

    use reqwest::Url;
    use sp_blockchain::HeaderBackend;
    use sp_runtime::traits::Block as BlockT;

    use super::*;

    pub async fn sync<B, C>(
        fetch_config: FetchConfig,
        sender_config: SenderConfig,
        rpc_port: u16,
        l1_url: Url,
        backend: Arc<mc_db::Backend<B>>,
        client: Arc<C>,
    ) where
        B: BlockT,
        C: HeaderBackend<B>,
    {
        let first_block = utility::get_last_synced_block(rpc_port).await + 1;

        let _ = tokio::join!(
            l1::sync(l1_url.clone()),
            l2::sync(sender_config, fetch_config.clone(), first_block, backend, client)
        );
    }
}

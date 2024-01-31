#![allow(deprecated)]

pub mod commitments;
pub mod l1;
pub mod l2;
pub mod types;
pub mod utils;

use std::sync::Arc;

pub use l2::{FetchConfig, SenderConfig};
use reqwest::Url;
use sp_runtime::traits::Block as BlockT;
use tokio::join;
pub use types::state_updates;
pub use utils::{convert, m, utility};

type CommandSink = futures::channel::mpsc::Sender<sc_consensus_manual_seal::rpc::EngineCommand<sp_core::H256>>;

pub struct StarknetSyncWorker<B: BlockT> {
    fetch_config: FetchConfig,
    sender_config: SenderConfig,
    rpc_port: u16,
    l1_url: Url,
    backend: Arc<mc_db::Backend<B>>,
}

impl<B: BlockT> StarknetSyncWorker<B> {
    pub fn new(
        fetch_config: FetchConfig,
        sender_config: SenderConfig,
        rpc_port: u16,
        l1_url: Url,
        backend: Arc<mc_db::Backend<B>>,
    ) -> Self {
        StarknetSyncWorker { fetch_config, sender_config, rpc_port, l1_url, backend }
    }

    pub async fn sync(&self) {
        let first_block = utility::get_last_synced_block(self.rpc_port).await + 1;

        let _ = tokio::join!(
            l1::sync(self.l1_url.clone(), self.rpc_port),
            l2::sync(self.sender_config.clone(), self.fetch_config.clone(), first_block, self.backend.clone())
        );
    }
}

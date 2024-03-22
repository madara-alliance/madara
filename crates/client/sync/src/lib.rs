#![allow(deprecated)]
#![feature(let_chains)]

// use std::sync::Arc;
// use sp_runtime::traits::Block as BlockT;
// use reqwest::Url;

pub mod commitments;
pub mod fetch;
pub mod l1;
pub mod l2;
pub mod reorgs;
pub mod types;
pub mod utils;

pub use deoxys_runtime::opaque::{DBlockT, DHashT};
pub use l2::SenderConfig;
pub use utils::{convert, m, utility};

type CommandSink = futures::channel::mpsc::Sender<sc_consensus_manual_seal::rpc::EngineCommand<sp_core::H256>>;

pub mod starknet_sync_worker {
    use std::sync::Arc;

    use mc_db::DeoxysBackend;
    use reqwest::Url;
    use sp_blockchain::HeaderBackend;

    use self::fetch::fetchers::FetchConfig;
    use super::*;

    pub async fn sync<C>(
        fetch_config: FetchConfig,
        sender_config: SenderConfig,
        rpc_port: u16,
        l1_url: Url,
        client: Arc<C>,
    ) where
        C: HeaderBackend<DBlockT>,
    {
        let first_block = utility::get_last_synced_block(rpc_port).await + 1;

        let _ = tokio::join!(
            l1::sync(l1_url.clone()),
            l2::sync(
                sender_config,
                fetch_config.clone(),
                first_block,
                DeoxysBackend::bonsai_contract(),
                DeoxysBackend::bonsai_storage(),
                DeoxysBackend::bonsai_class(),
                client,
            )
        );
    }
}

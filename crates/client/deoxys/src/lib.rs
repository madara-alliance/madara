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

use std::sync::Arc;

use bonsai_trie::{id::BasicId, BonsaiStorage, BonsaiStorageConfig};
pub use l2::{FetchConfig, SenderConfig};
use mc_db::{bonsai_db::{BonsaiConfigs, BonsaiDb}, BonsaiDbs};
use sp_runtime::traits::Block as BlockT;
use starknet_types_core::hash::{Pedersen, Poseidon};
pub use utils::{convert, m, utility};

type CommandSink = futures::channel::mpsc::Sender<sc_consensus_manual_seal::rpc::EngineCommand<sp_core::H256>>;

pub fn bonsai_configs<B: BlockT>(backend: Arc<mc_db::Backend<B>>) -> BonsaiConfigs<'static, B> {
    let config = BonsaiStorageConfig::default();

    let bonsai_dbs = BonsaiDbs {
        contract: Arc::clone(backend.bonsai_contract()),
        class: Arc::clone(backend.bonsai_class())
    };

    let mut contract: BonsaiStorage<BasicId, &BonsaiDb<B>, Pedersen> =
        BonsaiStorage::<_, _, Pedersen>::new(bonsai_dbs.contract.as_ref(), config).expect("Failed to create bonsai storage");
    let mut class =
        BonsaiStorage::<_, _, Poseidon>::new(bonsai_dbs.class.as_ref(), config).expect("Failed to create bonsai storage");

    BonsaiConfigs {
        contract,
        class,
    }
}


pub mod starknet_sync_worker {
    use std::sync::Arc;

    use reqwest::Url;
    use sp_runtime::traits::Block as BlockT;

    use super::*;

    pub async fn sync<B: BlockT>(
        fetch_config: FetchConfig,
        sender_config: SenderConfig,
        rpc_port: u16,
        l1_url: Url,
        backend: Arc<mc_db::Backend<B>>,
    ) {
        let first_block = utility::get_last_synced_block(rpc_port).await + 1;
        let bonsai_dbs = bonsai_configs(backend.clone());

        let _ = tokio::join!(
            l1::sync(l1_url.clone()),
            l2::sync(sender_config, fetch_config.clone(), first_block, rpc_port, bonsai_dbs)
        );
    }
}

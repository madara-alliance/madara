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

pub use l2::SenderConfig;
pub use mp_types::block::{DBlockT, DHashT};
pub use utils::{convert, m, utility};

type CommandSink = futures::channel::mpsc::Sender<sc_consensus_manual_seal::rpc::EngineCommand<sp_core::H256>>;

pub mod starknet_sync_worker {
    use std::sync::Arc;

    use mp_block::DeoxysBlock;
    use reqwest::Url;
    use sp_blockchain::HeaderBackend;
    use starknet_providers::sequencer::models::BlockId;
    use starknet_providers::SequencerGatewayProvider;
    use tokio::sync::mpsc::Sender;

    use self::fetch::fetchers::FetchConfig;
    use super::*;
    use crate::l2::verify_l2;

    pub async fn sync<C>(
        fetch_config: FetchConfig,
        block_sender: Sender<DeoxysBlock>,
        command_sink: CommandSink,
        l1_url: Url,
        client: Arc<C>,
        starting_block: u32,
    ) where
        C: HeaderBackend<DBlockT> + 'static,
    {
        let starting_block = starting_block + 1;

        let provider = SequencerGatewayProvider::new(
            fetch_config.gateway.clone(),
            fetch_config.feeder_gateway.clone(),
            fetch_config.chain_id,
            fetch_config.api_key,
        );

        if starting_block == 1 {
            let state_update =
                provider.get_state_update(BlockId::Number(0)).await.expect("getting state update for genesis block");
            verify_l2(0, &state_update).expect("verifying genesis block");
        }

        let _ = tokio::join!(
            l1::sync(l1_url.clone()),
            l2::sync(block_sender, command_sink, provider, starting_block.into(), fetch_config.verify, client)
        );
    }
}

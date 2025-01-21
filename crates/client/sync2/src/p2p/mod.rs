use crate::import::BlockImporter;
use mc_db::MadaraBackend;
use mc_p2p::P2pCommands;
use peer_set::PeerSet;
use std::sync::Arc;

mod classes;
mod events;
mod forward_sync;
mod headers;
mod peer_set;
mod pipeline;
mod state_diffs;
mod transactions;

pub use forward_sync::*;

#[derive(Clone)]
pub struct P2pPipelineArguments {
    pub(crate) backend: Arc<MadaraBackend>,
    pub(crate) peer_set: Arc<PeerSet>,
    pub(crate) p2p_commands: P2pCommands,
    pub(crate) importer: Arc<BlockImporter>,
}

impl P2pPipelineArguments {
    pub fn new(backend: Arc<MadaraBackend>, p2p_commands: P2pCommands, importer: Arc<BlockImporter>) -> Self {
        Self { importer, backend, peer_set: Arc::new(PeerSet::new(p2p_commands.clone())), p2p_commands }
    }
}

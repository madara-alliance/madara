use libp2p::{
    autonat, dcutr,
    gossipsub::{self, MessageAuthenticity},
    identify,
    identity::Keypair,
    kad::{self, store::MemoryStore},
    ping,
    relay::{self},
    swarm::NetworkBehaviour,
    StreamProtocol,
};
use mp_chain_config::ChainConfig;
use std::time::Duration;

use crate::sync_codec::codecs;

pub type Event = <MadaraP2pBehaviour as NetworkBehaviour>::ToSwarm;

#[derive(NetworkBehaviour)]
pub struct MadaraP2pBehaviour {
    /// Ping protocol.
    pub ping: ping::Behaviour,
    /// Kademlia is used for node discovery only.
    pub kad: kad::Behaviour<MemoryStore>,
    /// Identify as starknet node.
    pub identify: identify::Behaviour,

    /// Automatically make NAT configuration.
    pub autonat: autonat::Behaviour,
    /// DCUTR: Direct Connection Upgrade using Relay: this allows nodes behind a NAT to receive incoming connections through a relay node.
    pub dcutr: dcutr::Behaviour,
    /// If we're behind a NAT, we want to have a relay client to advertise a public address. It'll then be upgraded using DCUTR to a direct connection.
    pub relay: relay::client::Behaviour,

    /// Pubsub.
    pub gossipsub: gossipsub::Behaviour,

    // Single Req - Multiple Responses Streams
    pub headers_sync: p2p_stream::Behaviour<codecs::Headers>,
    pub classes_sync: p2p_stream::Behaviour<codecs::Classes>,
    pub state_diffs_sync: p2p_stream::Behaviour<codecs::StateDiffs>,
    pub transactions_sync: p2p_stream::Behaviour<codecs::Transactions>,
    pub events_sync: p2p_stream::Behaviour<codecs::Events>,
}

impl MadaraP2pBehaviour {
    // The return error type can't be anyhow::Error unfortunately because the SwarmBuilder won't let us
    pub fn new(
        chain_config: &ChainConfig,
        identity: &Keypair,
        relay_behaviour: libp2p::relay::client::Behaviour,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let pubkey = identity.public();
        let local_peer_id = pubkey.to_peer_id();

        let p2p_stream_config = p2p_stream::Config::default();
        Ok(Self {
            identify: identify::Behaviour::new(
                identify::Config::new(identify::PROTOCOL_NAME.to_string(), pubkey)
                    .with_agent_version(format!("madara/{}", env!("CARGO_PKG_VERSION"))),
            ),
            ping: Default::default(),
            kad: {
                let protocol = StreamProtocol::try_from_owned(format!("/starknet/kad/{}/1.0.0", chain_config.chain_id))
                    .expect("Invalid kad stream protocol");
                let mut cfg = kad::Config::new(protocol);
                const PROVIDER_PUBLICATION_INTERVAL: Duration = Duration::from_secs(600);
                cfg.set_record_ttl(Some(Duration::from_secs(0)));
                cfg.set_provider_record_ttl(Some(PROVIDER_PUBLICATION_INTERVAL * 3));
                cfg.set_provider_publication_interval(Some(PROVIDER_PUBLICATION_INTERVAL));
                cfg.set_periodic_bootstrap_interval(Some(Duration::from_millis(500)));
                cfg.set_query_timeout(Duration::from_secs(5 * 60));
                kad::Behaviour::with_config(local_peer_id, MemoryStore::new(local_peer_id), cfg)
            },
            autonat: autonat::Behaviour::new(local_peer_id, autonat::Config::default()),
            dcutr: dcutr::Behaviour::new(local_peer_id),
            relay: relay_behaviour,
            gossipsub: {
                let privacy = MessageAuthenticity::Signed(identity.clone());
                gossipsub::Behaviour::new(privacy, gossipsub::Config::default())
                    .map_err(|err| anyhow::anyhow!("Error making gossipsub config: {err}"))?
            },

            headers_sync: p2p_stream::Behaviour::with_codec(codecs::headers(), p2p_stream_config),
            classes_sync: p2p_stream::Behaviour::with_codec(codecs::classes(), p2p_stream_config),
            state_diffs_sync: p2p_stream::Behaviour::with_codec(codecs::state_diffs(), p2p_stream_config),
            transactions_sync: p2p_stream::Behaviour::with_codec(codecs::transactions(), p2p_stream_config),
            events_sync: p2p_stream::Behaviour::with_codec(codecs::events(), p2p_stream_config),
        })
    }
}

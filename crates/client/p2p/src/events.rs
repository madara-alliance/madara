//! Handle incomming p2p events
use crate::{
    behaviour::{self},
    MadaraP2p,
};
use libp2p::swarm::SwarmEvent;

impl MadaraP2p {
    pub fn handle_event(&mut self, event: SwarmEvent<behaviour::Event>) -> anyhow::Result<()> {
        tracing::info!("event: {event:?}");
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                let listen_address = address.with_p2p(*self.swarm.local_peer_id()).expect("Making multiaddr");
                tracing::info!("📡 Peer-to-peer listening on address {listen_address:?}");
            }
            SwarmEvent::Behaviour(behaviour::Event::Identify(libp2p::identify::Event::Received {
                peer_id,
                info,
                connection_id: _,
            })) => {
                tracing::info!("identify: {info:?}");
                // TODO: we may want to tell the local node about the info.observed_addr - but we probably need to check that address first
                // maybe we do want to trust the address if it comes from the relay..?
                // https://github.com/libp2p/rust-libp2p/blob/master/protocols/identify/CHANGELOG.md#0430
                // https://github.com/search?q=repo%3Alibp2p%2Frust-libp2p%20add_external_address&type=code
                self.swarm.add_external_address(info.observed_addr); // removing this will mean that the node won't switch to kad server mode and will stay client

                // check that we're supposed to be in the same network - we check that they have at least the kademlia protocol for our chain
                let local_kad_protocols = self.swarm.behaviour().kad.protocol_names();
                // local_kad_protocols=[/starknet/kad/SN_SEPOLIA/1.0.0]

                if !info.protocols.iter().any(|p| local_kad_protocols.contains(p)) {
                    // TODO: should we be more restrictive about this?
                    tracing::debug!(
                        "Got an Identify response from a peer ({peer_id}) that is not running any of our protocols"
                    );
                    return Ok(());
                }

                // Make kademlia aware of the identity of the peer we connected to.
                for addr in info.listen_addrs {
                    self.swarm.behaviour_mut().kad.add_address(&peer_id, addr);
                }
            }

            SwarmEvent::Behaviour(behaviour::Event::HeadersSync(event)) => {
                self.headers_sync_handler.handle_event(event);
            }
            _event => {}
        }
        Ok(())
    }
}

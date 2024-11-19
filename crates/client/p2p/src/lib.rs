use anyhow::Context;
use behaviour::SwarmBehaviour;
use libp2p::{futures::StreamExt, multiaddr::Protocol, swarm::SwarmEvent, Multiaddr};
use mp_utils::channel_wait_or_graceful_shutdown;

mod behaviour;

/// Protobuf messages.
pub mod model {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

pub struct P2pConfig {
    /// None to get an OS-assigned port.
    pub port: Option<u16>,
    pub bootstrap_nodes: Vec<Multiaddr>,
}

pub async fn launch_server(config: P2pConfig) -> anyhow::Result<()> {
    // we do not need to provide a stable identity at all
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            Default::default(),
            // support tls and noise
            (libp2p::tls::Config::new, libp2p::noise::Config::new),
            // multiplexing protocol (yamux)
            libp2p::yamux::Config::default,
        )
        .context("Confiurin libp2p tcp transport")?
        // .with_quic() // todo quic?
        .with_behaviour(|_| SwarmBehaviour::default())
        .context("Configuring libp2p behaviour")?
        .build();

    let multi_addr = "/ip4/0.0.0.0".parse::<Multiaddr>()?.with(Protocol::Tcp(config.port.unwrap_or(0)));
    swarm.listen_on(multi_addr).context("Binding port")?;

    for node in &config.bootstrap_nodes {
        if let Err(err) = swarm.dial(node.clone()) {
            tracing::debug!("Could not dial bootstrap node {node}: {err:#}");
        }
    }

    // let mut incoming_streams = swarm
    //     .behaviour()
    //     .new_control()
    //     .accept(ECHO_PROTOCOL)
    //     .unwrap();

    // Poll the swarm to make progress.
    while let Some(event) = channel_wait_or_graceful_shutdown(swarm.next()).await {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                let listen_address = address.with_p2p(*swarm.local_peer_id()).unwrap();
                tracing::info!("Peer-to-peer listening on adress {listen_address:?}");
            }
            event => tracing::trace!(?event),
        }
    }

    Ok(())
}

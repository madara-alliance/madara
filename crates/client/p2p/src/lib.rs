use anyhow::Context;
use behaviour::MadaraP2pBehaviour;
use futures::FutureExt;
use libp2p::{futures::StreamExt, multiaddr::Protocol, Multiaddr, Swarm};
use mc_db::MadaraBackend;
use mc_rpc::providers::AddTransactionProvider;
use mp_utils::graceful_shutdown;
use std::{sync::Arc, time::Duration};
use sync_handlers::DynSyncHandler;

mod behaviour;
mod events;
mod handlers_impl;
mod sync_codec;
mod sync_handlers;

/// Protobuf messages.
#[allow(clippy::all)]
pub mod model {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

pub struct P2pConfig {
    /// None to get an OS-assigned port.
    pub port: Option<u16>,
    pub bootstrap_nodes: Vec<Multiaddr>,
    pub status_interval: Duration,
}

#[derive(Clone)]
struct MadaraP2pContext {
    backend: Arc<MadaraBackend>,
}

pub struct MadaraP2p {
    config: P2pConfig,
    #[allow(unused)]
    db: Arc<MadaraBackend>,
    #[allow(unused)]
    add_transaction_provider: Arc<dyn AddTransactionProvider>,

    swarm: Swarm<MadaraP2pBehaviour>,

    headers_sync_handler: DynSyncHandler<MadaraP2pContext, model::BlockHeadersRequest, model::BlockHeadersResponse>,
}

impl MadaraP2p {
    pub fn new(
        config: P2pConfig,
        db: Arc<MadaraBackend>,
        add_transaction_provider: Arc<dyn AddTransactionProvider>,
    ) -> anyhow::Result<Self> {
        // we do not need to provide a stable identity except for bootstrap nodes
        let swarm = libp2p::SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                Default::default(),
                // support tls and noise
                (libp2p::tls::Config::new, libp2p::noise::Config::new),
                // multiplexing protocol (yamux)
                libp2p::yamux::Config::default,
            )
            .context("Configuring libp2p tcp transport")?
            .with_relay_client(libp2p::noise::Config::new, libp2p::yamux::Config::default)
            .context("Configuring relay transport")?
            .with_behaviour(|identity, relay_client| MadaraP2pBehaviour::new(db.chain_config(), identity, relay_client))
            .context("Configuring libp2p behaviour")?
            .build();

        let app_ctx = MadaraP2pContext { backend: Arc::clone(&db) };

        Ok(Self {
            config,
            db,
            add_transaction_provider,
            swarm,
            headers_sync_handler: DynSyncHandler::new("headers", app_ctx.clone(), |ctx, req, out| {
                handlers_impl::headers_sync(ctx, req, out).boxed()
            }),
        })
    }

    /// Main loop of the p2p service.
    pub async fn run(&mut self) -> anyhow::Result<()> {
        let multi_addr = "/ip4/0.0.0.0".parse::<Multiaddr>()?.with(Protocol::Tcp(self.config.port.unwrap_or(0)));
        self.swarm.listen_on(multi_addr).context("Binding port")?;

        for node in &self.config.bootstrap_nodes {
            if let Err(err) = self.swarm.dial(node.clone()) {
                tracing::debug!("Could not dial bootstrap node {node}: {err:#}");
            }
        }

        let mut status_interval = tokio::time::interval(self.config.status_interval);

        loop {
            tokio::select! {
                // Stop condition
                _ = graceful_shutdown() => break,

                // Show node status regularly
                _ = status_interval.tick() => {
                    let network_info = self.swarm.network_info();
                    let connections_info = network_info.connection_counters();

                    let peers = network_info.num_peers();
                    let connections_in = connections_info.num_established_incoming();
                    let connections_out = connections_info.num_established_outgoing();
                    let pending_connections = connections_info.num_pending();
                    let dht = self.swarm.behaviour_mut().kad
                        .kbuckets()
                        // Cannot .into_iter() a KBucketRef, hence the inner collect followed by flat_map
                        .map(|kbucket_ref| {
                            kbucket_ref
                                .iter()
                                .map(|entry_ref| *entry_ref.node.key.preimage())
                                .collect::<Vec<_>>()
                        })
                        .flat_map(|peers_in_bucket| peers_in_bucket.into_iter())
                        .collect::<std::collections::HashSet<_>>();
                    tracing::info!("P2P {peers} peers  IN: {connections_in}  OUT: {connections_out}  Pending: {pending_connections}");
                    tracing::info!("DHT {dht:?}");
                }

                // Handle incoming service commands
                // _ =

                // Make progress on the swarm and handle the events it yields
                event = self.swarm.next() => match event {
                    Some(event) => self.handle_event(event).context("Handling p2p event")?,
                    None => break,
                }
            }
        }
        Ok(())
    }
}

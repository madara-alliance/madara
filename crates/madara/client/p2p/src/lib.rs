use anyhow::Context;
use behaviour::MadaraP2pBehaviour;
use futures::{channel::mpsc, FutureExt};
use libp2p::{futures::StreamExt, gossipsub::IdentTopic, identity::Keypair, multiaddr::Protocol, Multiaddr, Swarm};
use mc_db::MadaraBackend;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use sync_handlers::DynSyncHandler;

mod behaviour;
mod commands;
mod events;
mod handlers_impl;
mod identity;
mod model_primitives;
mod sync_codec;
mod sync_handlers;

pub use commands::*;
pub use libp2p::PeerId;
pub use sync_handlers::Error as SyncHandlerError;

/// Protobuf messages.
#[allow(clippy::all)]
pub mod model {
    pub use crate::model_primitives::*;
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

pub struct P2pConfig {
    /// None to get an OS-assigned port.
    pub port: Option<u16>,
    pub bootstrap_nodes: Vec<Multiaddr>,
    pub status_interval: Duration,
    /// Peer-to-peer identity.json file. By default, we generate a new one everytime.
    pub identity_file: Option<PathBuf>,
    pub save_identity: bool,
}

#[derive(Clone)]
struct MadaraP2pContext {
    backend: Arc<MadaraBackend>,
}

pub struct MadaraP2pBuilder {
    commands: P2pCommands,
    commands_receiver: mpsc::Receiver<Command>,
    config: P2pConfig,
    db: Arc<MadaraBackend>,
    keypair: Keypair,
}

impl MadaraP2pBuilder {
    pub fn new(config: P2pConfig, db: Arc<MadaraBackend>) -> anyhow::Result<Self> {
        let (commands, commands_receiver) = mpsc::channel(100);

        // we do not need to provide a stable identity except for bootstrap nodes
        let keypair = identity::load_identity(config.identity_file.as_deref(), config.save_identity)?;

        Ok(Self {
            commands: P2pCommands { inner: commands, peer_id: keypair.public().to_peer_id() },
            commands_receiver,
            config,
            db,
            keypair,
        })
    }

    pub fn commands(&self) -> P2pCommands {
        self.commands.clone()
    }

    pub fn build(self) -> anyhow::Result<MadaraP2p> {
        let MadaraP2pBuilder { commands: _, commands_receiver, config, db, keypair } = self;

        let swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
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

        Ok(MadaraP2p {
            config,
            db,
            // add_transaction_provider,
            swarm,
            commands_receiver: Some(commands_receiver),
            pending_get_closest_peers: Default::default(),
            headers_sync_handler: DynSyncHandler::new("headers", app_ctx.clone(), |ctx, req, out| {
                handlers_impl::headers_sync(ctx, req, out).boxed()
            }),
            classes_sync_handler: DynSyncHandler::new("classes", app_ctx.clone(), |ctx, req, out| {
                handlers_impl::classes_sync(ctx, req, out).boxed()
            }),
            state_diffs_sync_handler: DynSyncHandler::new("state_diffs", app_ctx.clone(), |ctx, req, out| {
                handlers_impl::state_diffs_sync(ctx, req, out).boxed()
            }),
            transactions_sync_handler: DynSyncHandler::new("transactions", app_ctx.clone(), |ctx, req, out| {
                handlers_impl::transactions_sync(ctx, req, out).boxed()
            }),
            events_sync_handler: DynSyncHandler::new("events", app_ctx.clone(), |ctx, req, out| {
                handlers_impl::events_sync(ctx, req, out).boxed()
            }),
        })
    }
}

pub struct MadaraP2p {
    config: P2pConfig,
    #[allow(unused)]
    db: Arc<MadaraBackend>,
    // #[allow(unused)]
    // add_transaction_provider: Arc<dyn AddTransactionProvider>,
    commands_receiver: Option<mpsc::Receiver<Command>>,
    swarm: Swarm<MadaraP2pBehaviour>,

    pending_get_closest_peers: HashMap<libp2p::kad::QueryId, mpsc::UnboundedSender<PeerId>>,

    headers_sync_handler: DynSyncHandler<MadaraP2pContext, model::BlockHeadersRequest, model::BlockHeadersResponse>,
    classes_sync_handler: DynSyncHandler<MadaraP2pContext, model::ClassesRequest, model::ClassesResponse>,
    state_diffs_sync_handler: DynSyncHandler<MadaraP2pContext, model::StateDiffsRequest, model::StateDiffsResponse>,
    transactions_sync_handler:
        DynSyncHandler<MadaraP2pContext, model::TransactionsRequest, model::TransactionsResponse>,
    events_sync_handler: DynSyncHandler<MadaraP2pContext, model::EventsRequest, model::EventsResponse>,
}

impl MadaraP2p {
    pub fn dial_bootstrap_nodes(&mut self) {
        for node in &self.config.bootstrap_nodes {
            if let Err(err) = self.swarm.dial(node.clone()) {
                tracing::debug!("Could not dial bootstrap node {node}: {err:#}");
            }
        }
    }

    /// Main loop of the p2p service.
    pub async fn run(&mut self, mut ctx: mp_utils::service::ServiceContext) -> anyhow::Result<()> {
        let multi_addr = "/ip4/0.0.0.0".parse::<Multiaddr>()?.with(Protocol::Tcp(self.config.port.unwrap_or(0)));
        self.swarm.listen_on(multi_addr).context("Binding port")?;

        self.dial_bootstrap_nodes();

        let block_propagation_topic = &format!("blocks/{}", self.db.chain_config().chain_id.as_hex());
        self.swarm.behaviour_mut().gossipsub.subscribe(&IdentTopic::new(block_propagation_topic))?;
        tracing::debug!("Gossipsub subscribed to {block_propagation_topic}");

        let mut status_interval = tokio::time::interval(self.config.status_interval);
        let mut commands_recv = self.commands_receiver.take().context("Service already started")?;

        loop {
            tokio::select! {
                // Stop condition
                _ = ctx.cancelled() => break,

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
                    tracing::trace!("DHT {dht:?}");
                }

                // Make progress on the swarm and handle the events it yields
                event = self.swarm.next() => match event {
                    Some(event) => self.handle_event(event).context("Handling p2p event")?,
                    None => break,
                },

                // Handle incoming service commands
                Some(command) = commands_recv.next() => {
                    self.handle_command(command);
                }
            }
        }
        Ok(())
    }
}

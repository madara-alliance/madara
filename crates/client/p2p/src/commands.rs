use crate::{
    handlers_impl::{self},
    model, sync_handlers, MadaraP2p,
};
use futures::{
    channel::{mpsc, oneshot},
    SinkExt, Stream,
};
use libp2p::PeerId;
use mc_db::stream::BlockStreamConfig;
use mp_block::{BlockHeaderWithSignatures, TransactionWithReceipt};

#[derive(Debug, Clone)]
pub struct P2pCommands(pub(crate) mpsc::Sender<Command>);
impl P2pCommands {
    pub async fn get_random_peers(&mut self) -> Vec<PeerId> {
        let (callback, recv) = oneshot::channel();
        let _res = self.0.send(Command::GetRandomPeers { callback }).await;
        recv.await.unwrap_or(vec![])
    }
    fn make_stream<'a, T: 'static, R>(
        &self,
        debug_name: &'static str,
        peer: PeerId,
        recv: mpsc::Receiver<T>,
        f: impl Fn(T) -> Result<R, sync_handlers::Error> + 'a,
    ) -> impl Stream<Item = R> + 'a {
        tokio_stream::StreamExt::map_while(recv, move |res| match f(res) {
            Ok(res) => Some(res),
            Err(sync_handlers::Error::Internal(err)) => {
                tracing::error!(target: "p2p_errors", "Internal server error in stream {debug_name} [peer_id {peer}]: {err:#}");
                None
            }
            Err(sync_handlers::Error::BadRequest(err)) => {
                tracing::debug!(target: "p2p_errors", "Bad request in stream {debug_name} [peer_id {peer}]: {err:#}");
                None
            }
            Err(sync_handlers::Error::SenderClosed) => None,
        })
    }
    pub async fn make_headers_stream(
        &mut self,
        peer: PeerId,
        config: BlockStreamConfig,
    ) -> impl Stream<Item = BlockHeaderWithSignatures> + 'static {
        let (callback, recv) = mpsc::channel(3);
        let req = model::BlockHeadersRequest { iteration: Some(config.into()) };
        let _res = self.0.send(Command::SyncHeaders { peer, req, callback }).await;
        self.make_stream("headers", peer, recv, handlers_impl::map_header_response)
    }
    /// Note: The events in the transaction receipt will not be filled in. Use [`Self::make_events_stream`] to get them.
    pub async fn make_transactions_stream(
        &mut self,
        peer: PeerId,
        config: BlockStreamConfig,
    ) -> impl Stream<Item = TransactionWithReceipt> + 'static {
        let (callback, recv) = mpsc::channel(3);
        let req = model::TransactionsRequest { iteration: Some(config.into()) };
        let _res = self.0.send(Command::SyncTransactions { peer, req, callback }).await;
        self.make_stream("headers", peer, recv, handlers_impl::map_transaction_response)
    }
}

pub enum Command {
    GetRandomPeers {
        callback: oneshot::Sender<Vec<PeerId>>,
    },
    SyncHeaders {
        peer: PeerId,
        req: model::BlockHeadersRequest,
        callback: mpsc::Sender<model::BlockHeadersResponse>,
    },
    SyncClasses {
        peer: PeerId,
        req: model::ClassesRequest,
        callback: mpsc::Sender<model::ClassesResponse>,
    },
    SyncStateDiffs {
        peer: PeerId,
        req: model::StateDiffsRequest,
        callback: mpsc::Sender<model::StateDiffsResponse>,
    },
    SyncTransactions {
        peer: PeerId,
        req: model::TransactionsRequest,
        callback: mpsc::Sender<model::TransactionsResponse>,
    },
    SyncEvents {
        peer: PeerId,
        req: model::EventsRequest,
        callback: mpsc::Sender<model::EventsResponse>,
    },
}

impl MadaraP2p {
    pub fn handle_command(&mut self, command: Command) {
        match command {
            Command::GetRandomPeers { callback } => callback.send(vec![]).unwrap_or_default(),
            Command::SyncHeaders { peer, req, callback } => {
                let request_id = self.swarm.behaviour_mut().headers_sync.send_request(&peer, req);
                self.headers_sync_handler.add_outbound(request_id, callback);
            }
            Command::SyncClasses { peer, req, callback } => {
                let request_id = self.swarm.behaviour_mut().classes_sync.send_request(&peer, req);
                self.classes_sync_handler.add_outbound(request_id, callback);
            }
            Command::SyncStateDiffs { peer, req, callback } => {
                let request_id = self.swarm.behaviour_mut().state_diffs_sync.send_request(&peer, req);
                self.state_diffs_sync_handler.add_outbound(request_id, callback);
            }
            Command::SyncTransactions { peer, req, callback } => {
                let request_id = self.swarm.behaviour_mut().transactions_sync.send_request(&peer, req);
                self.transactions_sync_handler.add_outbound(request_id, callback);
            }
            Command::SyncEvents { peer, req, callback } => {
                let request_id = self.swarm.behaviour_mut().events_sync.send_request(&peer, req);
                self.events_sync_handler.add_outbound(request_id, callback);
            }
        }
    }
}

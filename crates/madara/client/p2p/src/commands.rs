use std::collections::{HashMap, HashSet};

use crate::{
    handlers_impl::{self},
    sync_handlers, MadaraP2p,
};
use futures::{channel::mpsc, stream, SinkExt, Stream, StreamExt};
use libp2p::PeerId;
use mc_db::stream::BlockStreamConfig;
use mp_block::{BlockHeaderWithSignatures, TransactionWithReceipt};
use mp_class::ClassInfoWithHash;
use mp_proto::model;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::{DeclaredClassCompiledClass, StateDiff};
use starknet_core::types::Felt;

#[derive(Debug, Clone)]
pub struct P2pCommands {
    pub(crate) inner: mpsc::Sender<Command>,
    pub(crate) peer_id: PeerId,
}

impl P2pCommands {
    pub async fn get_random_peers(&mut self) -> HashSet<PeerId> {
        let (callback, recv) = mpsc::unbounded();
        let _res = self.inner.send(Command::GetRandomPeers { callback }).await;
        recv.collect().await
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub async fn make_headers_stream(
        &mut self,
        peer: PeerId,
        config: BlockStreamConfig,
    ) -> impl Stream<Item = Result<BlockHeaderWithSignatures, sync_handlers::Error>> + 'static {
        let req = model::BlockHeadersRequest { iteration: Some(config.into()) };
        let (callback, recv) = mpsc::channel(3);
        let _res = self.inner.send(Command::SyncHeaders { peer, req, callback }).await;

        stream::unfold(recv, |mut recv| async move {
            let res = handlers_impl::read_headers_stream(recv.by_ref()).await;
            if matches!(res, Err(sync_handlers::Error::EndOfStream)) {
                return None;
            }
            Some((res, recv))
        })
    }

    /// Note: The events in the transaction receipt will not be filled in. Use [`Self::make_events_stream`] to get them.
    pub async fn make_transactions_stream<'a>(
        &mut self,
        peer: PeerId,
        config: BlockStreamConfig,
        transactions_count: impl IntoIterator<Item = usize> + 'a,
    ) -> impl Stream<Item = Result<Vec<TransactionWithReceipt>, sync_handlers::Error>> + 'a {
        let req = model::TransactionsRequest { iteration: Some(config.into()) };
        let (callback, recv) = mpsc::channel(3);
        let _res = self.inner.send(Command::SyncTransactions { peer, req, callback }).await;

        stream::unfold((recv, transactions_count.into_iter()), |(mut recv, mut transactions_count)| async move {
            let res = handlers_impl::read_transactions_stream(recv.by_ref(), transactions_count.next()?).await;
            if matches!(res, Err(sync_handlers::Error::EndOfStream)) {
                return None;
            }
            Some((res, (recv, transactions_count)))
        })
    }

    /// Note: The declared_contracts field of the state diff will be empty. Its content will be instead in the replaced class field.
    pub async fn make_state_diffs_stream<'a>(
        &mut self,
        peer: PeerId,
        config: BlockStreamConfig,
        state_diffs_length: impl IntoIterator<Item = usize> + 'a,
    ) -> impl Stream<Item = Result<StateDiff, sync_handlers::Error>> + 'a {
        let req = model::StateDiffsRequest { iteration: Some(config.into()) };
        let (callback, recv) = mpsc::channel(3);
        let _res = self.inner.send(Command::SyncStateDiffs { peer, req, callback }).await;

        stream::unfold((recv, state_diffs_length.into_iter()), |(mut recv, mut state_diffs_length)| async move {
            let res = handlers_impl::read_state_diffs_stream(recv.by_ref(), state_diffs_length.next()?).await;
            if matches!(res, Err(sync_handlers::Error::EndOfStream)) {
                return None;
            }
            Some((res, (recv, state_diffs_length)))
        })
    }

    pub async fn make_events_stream<'a>(
        &mut self,
        peer: PeerId,
        config: BlockStreamConfig,
        events_count: impl IntoIterator<Item = usize> + 'a,
    ) -> impl Stream<Item = Result<Vec<EventWithTransactionHash>, sync_handlers::Error>> + 'a {
        let req = model::EventsRequest { iteration: Some(config.into()) };
        let (callback, recv) = mpsc::channel(3);
        let _res = self.inner.send(Command::SyncEvents { peer, req, callback }).await;

        stream::unfold((recv, events_count.into_iter()), |(mut recv, mut events_count)| async move {
            let res = handlers_impl::read_events_stream(recv.by_ref(), events_count.next()?).await;
            if matches!(res, Err(sync_handlers::Error::EndOfStream)) {
                return None;
            }
            Some((res, (recv, events_count)))
        })
    }

    /// Note: you need to get the `declared_classes` from the `StateDiff`s beforehand.
    pub async fn make_classes_stream<'a>(
        &mut self,
        peer: PeerId,
        config: BlockStreamConfig,
        declared_classes: impl IntoIterator<Item = &HashMap<Felt, DeclaredClassCompiledClass>> + 'a,
    ) -> impl Stream<Item = Result<Vec<ClassInfoWithHash>, sync_handlers::Error>> + 'a {
        let req = model::ClassesRequest { iteration: Some(config.into()) };
        let (callback, recv) = mpsc::channel(3);
        let _res = self.inner.send(Command::SyncClasses { peer, req, callback }).await;

        stream::unfold((recv, declared_classes.into_iter()), |(mut recv, mut declared_classes)| async move {
            let res = handlers_impl::read_classes_stream(recv.by_ref(), declared_classes.next()?).await;
            if matches!(res, Err(sync_handlers::Error::EndOfStream)) {
                return None;
            }
            Some((res, (recv, declared_classes)))
        })
    }
}

#[derive(Debug)]
pub(crate) enum Command {
    GetRandomPeers {
        /// Channel is unbounded because we do not want the receiver to be able to block the p2p task.
        /// This is not an issue for the sync commands as their respective handlers are spawned as new tasks - thus handling
        /// backpressure.
        callback: mpsc::UnboundedSender<PeerId>,
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
    pub(crate) fn handle_command(&mut self, command: Command) {
        tracing::trace!("Handle command: {command:?}");
        match command {
            Command::GetRandomPeers { callback } => {
                let query_id = self.swarm.behaviour_mut().kad.get_closest_peers(PeerId::random());
                tracing::debug!("Started get random peers query: {query_id}");
                self.pending_get_closest_peers.insert(query_id, callback);
            }
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

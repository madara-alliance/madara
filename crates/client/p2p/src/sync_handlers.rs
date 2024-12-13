use futures::{channel::mpsc::Sender, future::BoxFuture, pin_mut, Future};
use p2p_stream::InboundRequestId;
use std::borrow::Cow;
use std::{collections::HashMap, fmt, marker::PhantomData};
use tokio::task::{AbortHandle, JoinSet};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Error is internal and will be reported with error level.
    #[error("Internal server error: {0:#}")]
    Internal(anyhow::Error),
    /// Error is the peer's fault, will only be reported with debug level.
    #[error("Bad request: {0}")]
    BadRequest(Cow<'static, str>),

    /// Sender closed. Do nothing.
    #[error("Channel closed")]
    SenderClosed(#[from] futures::channel::mpsc::SendError),
}

pub struct ReqContext<AppCtx: Clone> {
    pub app_ctx: AppCtx,
}

pub type DynSyncHandler<AppCtx, Req, Res> = StreamHandler<
    AppCtx,
    Req,
    Res,
    fn(ReqContext<AppCtx>, Req, Sender<Res>) -> BoxFuture<'static, Result<(), Error>>,
    BoxFuture<'static, Result<(), Error>>,
>;

pub struct StreamHandler<AppCtx, Req, Res, F, Fut> {
    debug_name: &'static str,
    app_ctx: AppCtx,
    handler: F,
    join_set: JoinSet<()>,
    current_inbound: HashMap<InboundRequestId, AbortHandle>,
    _boo: PhantomData<(Req, Res, Fut)>,
}

impl<AppCtx: Clone, Req: Send, Res: Send + 'static, F, Fut> StreamHandler<AppCtx, Req, Res, F, Fut>
where
    F: Fn(ReqContext<AppCtx>, Req, Sender<Res>) -> Fut,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    pub fn new(debug_name: &'static str, app_ctx: AppCtx, handler: F) -> Self {
        Self {
            debug_name,
            handler,
            app_ctx,
            join_set: Default::default(),
            current_inbound: Default::default(),
            _boo: PhantomData,
        }
    }

    pub fn handle_event(&mut self, ev: p2p_stream::Event<Req, Res>) {
        match ev {
            /* === OTHER PEER => US === */
            p2p_stream::Event::InboundRequest { request_id, request, peer, channel } => {
                tracing::debug!("New inbounds request in stream {} [peer_id {}]", self.debug_name, peer);
                let ctx = ReqContext { app_ctx: self.app_ctx.clone() };
                // Spawn the task that responds to the request.

                let fut = (self.handler)(ctx, request, channel);

                let abort_handle = self.join_set.spawn(async move {
                    let fut = fut;
                    pin_mut!(fut);

                    if let Err(err) = fut.await {
                        match err {
                            Error::Internal(err) => {
                                tracing::error!(target: "p2p_errors", "Internal Server Error: {:#}", err);
                            }
                            Error::BadRequest(err) => {
                                tracing::debug!(target: "p2p_errors", "Bad request: {:#}", err);
                            }
                            Error::SenderClosed(_) => { /* sender closed, do nothing */ }
                        }
                    }
                });

                self.current_inbound.insert(request_id, abort_handle);
            }
            p2p_stream::Event::InboundFailure { peer, request_id, error } => {
                tracing::debug!("Inbounds failure in stream {} [peer_id {}]: {:#}", self.debug_name, peer, error);
                if let Some(v) = self.current_inbound.remove(&request_id) {
                    v.abort();
                }
            }
            p2p_stream::Event::OutboundResponseStreamClosed { peer, request_id } => {
                tracing::debug!("End of stream {} [peer_id {}]", self.debug_name, peer);
                if let Some(v) = self.current_inbound.remove(&request_id) {
                    v.abort(); // abort if not yet aborted
                }
            }
            /* === US => OTHER PEER === */
            p2p_stream::Event::OutboundRequestSentAwaitingResponses { .. } => todo!(),
            p2p_stream::Event::OutboundFailure { .. } => todo!(),
            p2p_stream::Event::InboundResponseStreamClosed { .. } => todo!(),
        }
    }
}

impl<AppCtx, Req, Res, F, S> fmt::Debug for StreamHandler<AppCtx, Req, Res, F, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StreamHandler[{}] <{} inbounds tasks>", self.debug_name, self.current_inbound.len())
    }
}

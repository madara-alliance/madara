use crate::eth::StarknetCoreContract::LogMessageToL2;
use crate::messaging::sync::CommonMessagingEventData;
use alloy::contract::EventPoller;
use alloy::rpc::types::Log;
use alloy::sol_types::SolEvent;
use alloy::transports::http::{Client, Http};
use anyhow::Error;
use futures::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct EthereumEventStream {
    stream: Pin<Box<dyn Stream<Item = Vec<Log>> + Send + 'static>>,
}

impl EthereumEventStream {
    pub fn new(watcher: EventPoller<Http<Client>, LogMessageToL2>) -> Self {
        Self { stream: Box::pin(watcher.poller.into_stream()) }
    }
}

impl Stream for EthereumEventStream {
    type Item = Option<anyhow::Result<CommonMessagingEventData>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(logs)) => {
                if let Some(log) = logs.into_iter().next() {
                    match LogMessageToL2::decode_log(log.as_ref(), false) {
                        Ok(event) => Poll::Ready(Some(Some(Ok(CommonMessagingEventData {
                            from: event.data.fromAddress.as_slice().into(),
                            to: event.data.toAddress.to_be_bytes_vec(),
                            selector: event.data.selector.to_be_bytes_vec(),
                            nonce: event.nonce.to_be_bytes_vec(),
                            payload: {
                                let mut payload_vec = vec![];
                                event.payload.iter().for_each(|ele| payload_vec.push(ele.to_be_bytes_vec()));
                                payload_vec
                            },
                            fee: Some(event.data.fee.to_be_bytes_vec()),
                            transaction_hash: Some(
                                log.transaction_hash.expect("Unable to get transaction hash from event log.").to_vec(),
                            ),
                            message_hash: None,
                            block_number: Some(log.block_number.expect("Unable to get block number from event log.")),
                            event_index: Some(log.log_index.expect("Unable to get log index from event log.")),
                        })))),
                        Err(e) => Poll::Ready(Some(Some(Err(Error::from(e))))),
                    }
                } else {
                    Poll::Ready(None)
                }
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

use crate::messaging::sync::CommonMessagingEventData;
use futures::Stream;
use starknet_core::types::{BlockId, EmittedEvent, EventFilter};
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::{JsonRpcClient, Provider};
use starknet_types_core::felt::Felt;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::time::sleep;

type FutureType = Pin<Box<dyn Future<Output = anyhow::Result<(Option<EmittedEvent>, EventFilter)>> + Send>>;

pub struct StarknetEventStream {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    filter: EventFilter,
    processed_events: HashSet<Felt>,
    future: Option<FutureType>,
    polling_interval: Duration,
}

impl StarknetEventStream {
    pub fn new(provider: Arc<JsonRpcClient<HttpTransport>>, filter: EventFilter, polling_interval: Duration) -> Self {
        Self { provider, filter, processed_events: HashSet::new(), future: None, polling_interval }
    }
}

impl Stream for StarknetEventStream {
    type Item = Option<anyhow::Result<CommonMessagingEventData>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.future.is_none() {
            let provider = self.provider.clone();
            let filter = self.filter.clone();
            let processed_events = self.processed_events.clone();
            let polling_interval = self.polling_interval;

            async fn fetch_events(
                provider: Arc<JsonRpcClient<HttpTransport>>,
                mut filter: EventFilter,
                processed_events: HashSet<Felt>,
                polling_interval: Duration,
            ) -> anyhow::Result<(Option<EmittedEvent>, EventFilter)> {
                // Adding sleep to introduce delay
                sleep(polling_interval).await;

                let mut event_vec = Vec::new();
                let mut page_indicator = false;
                let mut continuation_token: Option<String> = None;

                while !page_indicator {
                    let events = provider
                        .get_events(
                            EventFilter {
                                from_block: filter.from_block,
                                to_block: filter.to_block,
                                address: filter.address,
                                keys: filter.keys.clone(),
                            },
                            continuation_token.clone(),
                            1000,
                        )
                        .await?;

                    event_vec.extend(events.events);
                    if let Some(token) = events.continuation_token {
                        continuation_token = Some(token);
                    } else {
                        page_indicator = true;
                    }
                }

                for event in event_vec.clone() {
                    let event_nonce = event.data[4];
                    if !processed_events.contains(&event_nonce) {
                        return Ok((Some(event), filter));
                    }
                }

                let latest_block = provider.block_number().await?;
                filter.from_block = filter.to_block;
                filter.to_block = Some(BlockId::Number(latest_block));

                Ok((None, filter))
            }

            let future = async move {
                let (event, updated_filter) =
                    fetch_events(provider, filter, processed_events, polling_interval).await?;
                Ok((event, updated_filter))
            };

            self.future = Some(Box::pin(future));
        }

        // Poll the future
        let fut = self.future.as_mut().unwrap();
        match fut.as_mut().poll(cx) {
            Poll::Ready(result) => {
                self.future = None;
                match result {
                    Ok((Some(event), updated_filter)) => {
                        // Update the filter
                        self.filter = updated_filter;
                        // Insert the event nonce before returning
                        self.processed_events.insert(event.data[4]);

                        Poll::Ready(Some(Some(Ok(CommonMessagingEventData {
                            from: event.data[1].to_bytes_be().to_vec(),
                            to: event.data[2].to_bytes_be().to_vec(),
                            selector: event.data[3].to_bytes_be().to_vec(),
                            nonce: event.data[4].to_bytes_be().to_vec(),
                            payload: {
                                let mut payload_array = vec![];
                                event.data.iter().skip(6).for_each(|data| {
                                    payload_array.push(data.to_bytes_be().to_vec());
                                });
                                payload_array
                            },
                            fee: None,
                            transaction_hash: Some(event.transaction_hash.to_bytes_be().to_vec()),
                            message_hash: Some(event.data[0].to_bytes_be().to_vec()),
                            block_number: Some(event.block_number.expect("Unable to get block number from event.")),
                            event_index: None,
                        }))))
                    }
                    Ok((None, updated_filter)) => {
                        // Update the filter even when no events are found
                        self.filter = updated_filter;
                        Poll::Ready(Some(None))
                    }
                    Err(e) => Poll::Ready(Some(Some(Err(e)))),
                }
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

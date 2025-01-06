use crate::messaging::sync::CommonMessagingEventData;
use bigdecimal::ToPrimitive;
use futures::{Stream, TryFuture};
use starknet_core::types::{BlockId, EmittedEvent, EventFilter};
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::{JsonRpcClient, Provider};
use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

#[derive(Hash, Eq, PartialEq)]
struct EventKey {
    block_number: u64,
    transaction_hash: String,
    event_index: u64,
}

impl From<&EmittedEvent> for EventKey {
    fn from(event: &EmittedEvent) -> Self {
        Self {
            block_number: event.block_number.unwrap_or(0),
            transaction_hash: event.transaction_hash.to_string(),
            event_index: event.data[4].to_u64().unwrap(), // nonce of the event
        }
    }
}

pub struct StarknetEventStream {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    filter: EventFilter,
    processed_events: HashSet<EventKey>,
}

impl StarknetEventStream {
    pub fn new(provider: Arc<JsonRpcClient<HttpTransport>>, filter: EventFilter) -> Self {
        Self { provider, filter, processed_events: HashSet::new() }
    }

    fn is_duplicate(&self, event: &EmittedEvent) -> bool {
        self.processed_events.contains(&EventKey::from(event))
    }

    async fn get_events(&self) -> anyhow::Result<Vec<EmittedEvent>> {
        let mut event_vec = Vec::new();
        let mut page_indicator = false;
        let mut continuation_token: Option<String> = None;

        while !page_indicator {
            let events = self
                .provider
                .get_events(
                    EventFilter {
                        from_block: self.filter.from_block,
                        to_block: self.filter.to_block,
                        address: self.filter.address,
                        keys: self.filter.keys.clone(),
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

        Ok(event_vec)
    }
}

impl Stream for StarknetEventStream {
    type Item = Option<anyhow::Result<CommonMessagingEventData>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let future = async {
            let events = self.get_events().await?;
            let latest_block = self.provider.block_number().await?;
            self.filter.from_block = Some(BlockId::Number(latest_block));
            self.filter.to_block = Some(BlockId::Number(latest_block));

            for event in events {
                if !self.is_duplicate(&event) {
                    let key = EventKey::from(&event);
                    self.processed_events.insert(key);
                    return Ok(Some(event));
                }
            }

            Ok(None)
        };

        match futures::ready!(Box::pin(future).as_mut().try_poll(cx)) {
            Ok(Some(event)) => Poll::Ready(Some(Some(Ok(CommonMessagingEventData {
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
                transaction_hash: None,
                message_hash: Some(event.data[0].to_bytes_be().to_vec()),
                block_number: None,
                event_index: None,
            })))),
            Ok(None) => Poll::Ready(None),
            Err(e) => Poll::Ready(Some(Some(Err(e)))),
        }
    }
}

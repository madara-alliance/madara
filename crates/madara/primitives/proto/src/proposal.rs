use std::{
    collections::{btree_map, BTreeMap},
    sync::{Arc, Mutex},
    task::Poll,
};

use m_proc_macros::model_describe;

use crate::{
    model::{self, stream_message},
    model_field,
};

#[derive(thiserror::Error, Debug)]
pub enum StreamReceiverError<StreamId>
where
    StreamId: std::fmt::Debug,
{
    #[error("Invalid stream id: expected {0:?}, got {1:?}")]
    InvalidStreamId(StreamId, StreamId),
    #[error("The stream has lagged too far behind")]
    LagCap, // TODO: implement this
    #[error("Received message but already received Fin before it")]
    SendAfterFin,
    #[error("New Fin with id {0} but already received Fin at message id {1}")]
    DoubleFin(u64, u64),
    #[error("Message with id {0} has already been received")]
    DoubleSend(u64),
    #[error("Failed to decode model: {0}")]
    DecodeError(#[from] prost::DecodeError),
    #[error(transparent)]
    ModelError(#[from] crate::FromModelError),
    #[error("Channel was closed over an unfinished stream")]
    ChannelError,
}

type Never = Option<std::convert::Infallible>;
pub type ConsensusStreamBuilder = StreamBuilder<model::ProposalPart, model::ConsensusStreamId>;

pub struct StreamBuilder<Message, StreamId, T = Never, V = Never>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + Default + std::fmt::Debug,
{
    lag_cap: T,
    channel_cap: V,
    stream_id: Option<Vec<u8>>,
    _phantom1: std::marker::PhantomData<Message>,
    _phantom2: std::marker::PhantomData<StreamId>,
}

pub struct OrderedStreamSender {
    sender: tokio::sync::mpsc::Sender<model::StreamMessage>,
    waker: Arc<Mutex<Option<std::task::Waker>>>,
}

pub struct OrderedStreamReceiver<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + Default + std::fmt::Debug,
{
    receiver: tokio::sync::mpsc::Receiver<model::StreamMessage>,
    state: AccumulatorStateMachine<Message, StreamId>,
    waker: Arc<Mutex<Option<std::task::Waker>>>,
}

#[derive(Debug, Default)]
#[cfg_attr(test, derive(Clone))]
enum AccumulatorStateMachine<Message, StreamId>
where
    Message: prost::Message + Default,
    StreamId: prost::Message + Default + std::fmt::Debug,
{
    Accumulate(AccumulatorStateInner<Message, StreamId>),
    #[default]
    Done,
}

#[cfg_attr(test, derive(Clone))]
struct AccumulatorStateInner<Message, StreamId>
where
    Message: prost::Message + Default,
    StreamId: prost::Message + Default + std::fmt::Debug,
{
    messages: BTreeMap<u64, Message>,
    stream_id: Option<Vec<u8>>,
    fin: Option<u64>,
    id_low: u64,
    // TODO: implement this
    lag_cap: u64,
    _phantom: std::marker::PhantomData<StreamId>,
}

impl<Message, StreamId> std::fmt::Debug for OrderedStreamReceiver<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + Default + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.state {
            AccumulatorStateMachine::Accumulate(..) => {
                f.debug_struct("OrderedStreamAccumulator").field("state", &"Accumulating").finish()
            }
            AccumulatorStateMachine::Done { .. } => {
                f.debug_struct("OrderedStreamAccumulator").field("state", &"Done").finish()
            }
        }
    }
}

impl<Message, StreamId> std::fmt::Debug for AccumulatorStateInner<Message, StreamId>
where
    Message: prost::Message + Default,
    StreamId: prost::Message + Default + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(stream_id) = &self.stream_id {
            if let Ok(stream_id) = StreamId::decode(stream_id.as_slice()) {
                return f
                    .debug_struct("AccumulatorStateInner")
                    .field("messages", &format!("... ({})", self.messages.len()))
                    .field("stream_id", &stream_id)
                    .field("id_low", &self.id_low)
                    .field("lag_cap", &self.lag_cap)
                    .field("fin", &self.fin)
                    .finish();
            }
        }

        f.debug_struct("AccumulatorStateInner")
            .field("messages", &format!("... ({})", self.messages.len()))
            .field("stream_id", &"None")
            .field("id_low", &self.id_low)
            .field("lag_cap", &self.lag_cap)
            .field("fin", &self.fin)
            .finish()
    }
}

impl<T, V, Message, StreamId> StreamBuilder<Message, StreamId, T, V>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + Default + std::fmt::Debug,
{
    pub fn new() -> StreamBuilder<Message, StreamId, Never, Never> {
        StreamBuilder {
            lag_cap: None,
            channel_cap: None,
            stream_id: None,
            _phantom1: std::marker::PhantomData,
            _phantom2: std::marker::PhantomData,
        }
    }

    pub fn with_lag_cap(self, lag_cap: u64) -> StreamBuilder<Message, StreamId, u64, V> {
        let Self { channel_cap, stream_id, _phantom1, _phantom2, .. } = self;
        StreamBuilder { lag_cap, channel_cap, stream_id, _phantom1, _phantom2 }
    }

    pub fn with_channel_cap(self, channel_cap: usize) -> StreamBuilder<Message, StreamId, T, usize> {
        let Self { lag_cap, stream_id, _phantom1, _phantom2, .. } = self;
        StreamBuilder { lag_cap, channel_cap, stream_id, _phantom1, _phantom2 }
    }

    pub fn with_stream_id(self, stream_id: &[u8]) -> StreamBuilder<Message, StreamId, T, V> {
        let Self { lag_cap, channel_cap, _phantom1, _phantom2, .. } = self;
        Self { stream_id: Some(stream_id.to_vec()), lag_cap, channel_cap, _phantom1, _phantom2 }
    }
}

impl<Message, StreamId> StreamBuilder<Message, StreamId, u64, usize>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + Default + std::fmt::Debug,
{
    pub fn build(self) -> (OrderedStreamSender, OrderedStreamReceiver<Message, StreamId>) {
        let Self { lag_cap, channel_cap, stream_id, .. } = self;
        let (sx, rx) = tokio::sync::mpsc::channel(channel_cap);
        let waker = Arc::new(Mutex::new(None));
        let state = AccumulatorStateMachine::new(lag_cap, stream_id);

        let sender = OrderedStreamSender { sender: sx, waker: Arc::clone(&waker) };
        let receiver = OrderedStreamReceiver { receiver: rx, state, waker };

        (sender, receiver)
    }
}

impl OrderedStreamSender {
    pub async fn send(
        &mut self,
        message: model::StreamMessage,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<model::StreamMessage>> {
        self.sender.send(message).await?;

        if let Some(waker) = self.waker.lock().expect("Poisoned lock").take() {
            waker.wake();
        }

        Ok(())
    }
}

impl<Message, StreamId> futures::Stream for OrderedStreamReceiver<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + Default,
{
    type Item = Result<Message, StreamReceiverError<StreamId>>;

    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Option<Self::Item>> {
        match std::mem::take(&mut self.state) {
            AccumulatorStateMachine::Accumulate(mut inner) => {
                if let Some(message) = inner.next_ordered() {
                    self.update(inner);
                    return Poll::Ready(Some(Ok(message)));
                }

                loop {
                    let stream_message = match self.receiver.poll_recv(cx) {
                        Poll::Ready(Some(stream_mesage)) => stream_mesage,
                        Poll::Ready(None) => return Poll::Ready(Some(Err(StreamReceiverError::ChannelError))),
                        Poll::Pending => break,
                    };

                    inner = match inner.accumulate(stream_message) {
                        Ok(inner) => inner,
                        Err(e) => return Poll::Ready(Some(Err(e))),
                    };
                }

                if inner.is_done() {
                    self.receiver.close();
                    return Poll::Ready(None);
                }

                let Some(message) = inner.next_ordered() else {
                    let _ = self.waker.lock().expect("Poisoned lock").insert(cx.waker().clone());
                    return Poll::Pending;
                };

                self.state = AccumulatorStateMachine::Accumulate(inner);
                Poll::Ready(Some(Ok(message)))
            }
            AccumulatorStateMachine::Done => Poll::Ready(None),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.state {
            AccumulatorStateMachine::Accumulate(ref inner) => {
                let lo = inner.messages.last_key_value().map(|(id, _)| *id - inner.id_low + 1).unwrap_or(1) as usize;
                let hi = inner.fin.map(|id| id as usize);
                (lo, hi)
            }
            AccumulatorStateMachine::Done => (0, Some(0)),
        }
    }
}

impl<Message, StreamId> OrderedStreamReceiver<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + Default,
{
    fn update(&mut self, inner: AccumulatorStateInner<Message, StreamId>) {
        if !inner.is_done() {
            self.state = AccumulatorStateMachine::Accumulate(inner)
        } else {
            self.receiver.close()
        }
    }
}

impl<Message, StreamId> AccumulatorStateMachine<Message, StreamId>
where
    Message: prost::Message + Default,
    StreamId: prost::Message + Default,
{
    fn new(lag_cap: u64, stream_id: Option<Vec<u8>>) -> Self {
        Self::Accumulate(AccumulatorStateInner {
            messages: Default::default(),
            stream_id,
            fin: None,
            id_low: 0,
            lag_cap,
            _phantom: std::marker::PhantomData,
        })
    }
}

impl<Message, StreamId> AccumulatorStateInner<Message, StreamId>
where
    Message: prost::Message + Default,
    StreamId: prost::Message + Default + std::fmt::Debug,
{
    #[model_describe(model::StreamMessage)]
    fn accumulate_with_force(
        mut self,
        stream_message: model::StreamMessage,
        force: bool,
    ) -> Result<Self, StreamReceiverError<StreamId>> {
        let stream_id = self.stream_id.get_or_insert_with(|| stream_message.stream_id.clone());
        let message_id = stream_message.message_id;

        Self::check_stream_id(&stream_message.stream_id, stream_id)?;
        Self::check_message_id(stream_message.message_id, self.fin)?;

        match model_field!(stream_message => message) {
            stream_message::Message::Content(bytes) => self.update_content(message_id, &bytes, force),
            stream_message::Message::Fin(..) => self.update_fin(message_id),
        }
    }

    fn accumulate(self, stream_message: model::StreamMessage) -> Result<Self, StreamReceiverError<StreamId>> {
        self.accumulate_with_force(stream_message, true)
    }

    fn check_stream_id(actual: &[u8], expected: &[u8]) -> Result<(), StreamReceiverError<StreamId>> {
        if actual != expected {
            let actual = StreamId::decode(actual)?;
            let expected = StreamId::decode(expected)?;
            Err(StreamReceiverError::InvalidStreamId(actual, expected))
        } else {
            Ok(())
        }
    }

    fn check_message_id(message_id: u64, fin: Option<u64>) -> Result<(), StreamReceiverError<StreamId>> {
        if let Some(fin) = fin {
            if message_id >= fin {
                return Err(StreamReceiverError::SendAfterFin);
            }
        }
        Ok(())
    }

    fn check_limit(message_id: u64, last_id: Option<u64>, lag_cap: u64) -> Result<(), StreamReceiverError<StreamId>> {
        if let Some(last_id) = last_id {
            let lag = message_id.abs_diff(last_id);
            if lag > lag_cap {
                return Err(StreamReceiverError::LagCap);
            }
        }
        Ok(())
    }

    fn update_content(
        mut self,
        message_id: u64,
        bytes: &[u8],
        force: bool,
    ) -> Result<Self, StreamReceiverError<StreamId>> {
        if !force {
            let last_id = self.messages.last_key_value().map(|(k, _)| *k);
            Self::check_limit(message_id, last_id, self.lag_cap)?;
        }

        match self.messages.entry(message_id) {
            btree_map::Entry::Vacant(entry) => entry.insert(Message::decode(bytes)?),
            btree_map::Entry::Occupied(_) => return Err(StreamReceiverError::DoubleSend(message_id)),
        };

        Ok(self)
    }

    fn update_fin(mut self, fin_id: u64) -> Result<Self, StreamReceiverError<StreamId>> {
        self.fin = match self.fin {
            Some(fin_id_old) if fin_id_old != fin_id => return Err(StreamReceiverError::DoubleFin(fin_id, fin_id_old)),
            _ => Some(fin_id),
        };

        if let Some((last_id, _)) = self.messages.last_key_value() {
            if *last_id >= fin_id {
                return Err(StreamReceiverError::SendAfterFin);
            }
        }

        Ok(self)
    }

    fn next_ordered(&mut self) -> Option<Message> {
        match self.messages.first_entry() {
            Some(entry) => {
                if *entry.key() == self.id_low {
                    self.id_low += 1;
                    Some(entry.remove())
                } else {
                    None
                }
            }
            None => None,
        }
    }

    fn is_done(&self) -> bool {
        if let Some(fin) = self.fin {
            self.id_low == fin
        } else {
            false
        }
    }
}

#[cfg(test)]
mod test {
    use futures::StreamExt;
    use prost::Message;
    use starknet_core::types::Felt;

    use crate::{
        model::{self},
        proposal::ConsensusStreamBuilder,
    };

    const STREAM_LEN: u32 = 10;

    fn model_encode<M>(proposal_part: M) -> Vec<u8>
    where
        M: prost::Message + Default,
    {
        let mut buffer = Vec::with_capacity(proposal_part.encoded_len());
        proposal_part.encode(&mut buffer).expect("Failed to encode model");

        buffer
    }

    #[rstest::fixture]
    fn proposal_part(#[default(0)] seed: u32) -> model::ProposalPart {
        model::ProposalPart {
            messages: Some(model::proposal_part::Messages::Init(model::ProposalInit {
                height: seed as u64,
                round: seed + 1,
                valid_round: Some(seed + 2),
                proposer: Some(model::Address(Felt::from(seed) + Felt::ONE)),
            })),
        }
    }

    #[rstest::fixture]
    fn stream_proposal_part(#[default(1)] len: u32) -> impl Iterator<Item = model::stream_message::Message> {
        (0..len)
            .map(proposal_part)
            .map(model_encode)
            .map(model::stream_message::Message::Content)
            .chain(std::iter::once(model::stream_message::Message::Fin(model::Fin {})))
    }

    //     #[rstest::fixture]
    //     fn stream_proposal_part_shuffled(
    //         stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
    //     ) -> impl Iterator<Item = model::stream_message::Message> {
    //         let mut rng = rand::rngs::SmallRng::seed_from_u64(42);
    //         let mut messages = stream_proposal_part.collect::<Vec<_>>();
    //         messages.shuffle(&mut rng);
    //         return messages.into_iter();
    //     }

    #[rstest::fixture]
    fn stream_id(#[default(0)] seed: u32) -> Vec<u8> {
        let stream_id = model::ConsensusStreamId { height: seed as u64, round: seed + 1 };
        model_encode(stream_id)
    }

    #[rstest::fixture]
    fn stream_messages(
        #[with(STREAM_LEN)] stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
        #[with(1)] stream_id: Vec<u8>,
    ) -> impl Iterator<Item = model::StreamMessage> {
        stream_proposal_part.enumerate().map(move |(i, message)| model::StreamMessage {
            message: Some(message),
            stream_id: stream_id.clone(),
            message_id: i as u64,
        })
    }

    #[rstest::fixture]
    fn stream_messages_rev(
        stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) -> impl Iterator<Item = model::StreamMessage> {
        stream_messages.collect::<Vec<_>>().into_iter().rev()
    }

    //     #[rstest::fixture]
    //     fn stream_message_shuffled(
    //         stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
    //         #[with(1)] stream_id: Vec<u8>,
    //     ) -> impl Iterator<Item = model::StreamMessage> {
    //         let mut rng = rand::rngs::SmallRng::seed_from_u64(42);
    //         let mut stream_messages = stream_proposal_part
    //             .enumerate()
    //             .map(move |(i, message)| model::StreamMessage {
    //                 message: Some(message),
    //                 stream_id: stream_id.clone(),
    //                 message_id: i as u64,
    //             })
    //             .collect::<Vec<_>>();
    //         stream_messages.shuffle(&mut rng);
    //
    //         stream_messages.into_iter()
    //     }
    //
    //     #[rstest::fixture]
    //     fn stream_message_invalid_stream_id(
    //         mut stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
    //         #[from(stream_id)]
    //         #[with(1)]
    //         stream_id_1: Vec<u8>,
    //         #[from(stream_id)]
    //         #[with(2)]
    //         stream_id_2: Vec<u8>,
    //     ) -> impl Iterator<Item = model::StreamMessage> {
    //         vec![
    //             stream_proposal_part
    //                 .next()
    //                 .map(|message| model::StreamMessage { message: Some(message), stream_id: stream_id_1, message_id: 0 })
    //                 .expect("Failed to generate stream message"),
    //             stream_proposal_part
    //                 .next()
    //                 .map(|message| model::StreamMessage { message: Some(message), stream_id: stream_id_2, message_id: 0 })
    //                 .expect("Failed to generate stream message"),
    //         ]
    //         .into_iter()
    //     }
    //
    //     #[rstest::fixture]
    //     fn stream_message_double_fin(
    //         stream_message: impl Iterator<Item = model::StreamMessage>,
    //         #[with(1)] stream_id: Vec<u8>,
    //     ) -> impl Iterator<Item = model::StreamMessage> {
    //         std::iter::once(model::StreamMessage {
    //             message: Some(model::stream_message::Message::Fin(model::Fin {})),
    //             stream_id: stream_id.clone(),
    //             message_id: u64::MAX,
    //         })
    //         .chain(stream_message)
    //         .map(move |mut stream_message| {
    //             stream_message.stream_id = stream_id.clone();
    //             stream_message
    //         })
    //     }
    //
    //     #[rstest::fixture]
    //     fn stream_message_decode_error(
    //         mut stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
    //         #[with(1)] stream_id: Vec<u8>,
    //     ) -> impl Iterator<Item = model::StreamMessage> {
    //         std::iter::once(model::StreamMessage {
    //             message: Some(stream_proposal_part.next().unwrap()),
    //             stream_id: stream_id.clone(),
    //             message_id: 0,
    //         })
    //         .chain(std::iter::once(model::StreamMessage {
    //             message: Some(model::stream_message::Message::Fin(model::Fin {})),
    //             stream_id: stream_id.clone(),
    //             message_id: 0,
    //         }))
    //     }
    //
    //     #[rstest::fixture]
    //     fn stream_message_model_error(#[with(1)] stream_id: Vec<u8>) -> impl Iterator<Item = model::StreamMessage> {
    //         std::iter::once(model::StreamMessage { message: None, stream_id, message_id: 0 })
    //     }

    /// Receives a proposal part in a single, ordered stream. All should work as
    /// expected
    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_simple(stream_messages: impl Iterator<Item = model::StreamMessage>) {
        let (mut stream_sender, mut stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(STREAM_LEN as u64)
            .with_channel_cap((STREAM_LEN + 1) as usize)
            .build();

        for message in stream_messages {
            stream_sender.send(message.clone()).await.expect("Failed to send stream message");
            let next = stream_receiver.next().await;

            match message.message.unwrap() {
                model::stream_message::Message::Content(bytes) => {
                    let proposal_part =
                        model::ProposalPart::decode(bytes.as_slice()).expect("Failed to decode proposal part");
                    assert_matches::assert_matches!(next, Some(Ok(..)));
                    assert_eq!(proposal_part, next.unwrap().unwrap());
                }
                model::stream_message::Message::Fin(_) => {
                    assert!(next.is_none());
                }
            }
        }

        assert!(stream_receiver.next().await.is_none());
    }

    /// Receives a proposal part in a single, reversed stream. All should work as
    /// expected
    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_reversed(
        stream_messages: impl Iterator<Item = model::StreamMessage>,
        stream_messages_rev: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (mut stream_sender, mut stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(STREAM_LEN as u64)
            .with_channel_cap((STREAM_LEN + 1) as usize)
            .build();

        for message in stream_messages_rev {
            stream_sender.send(message.clone()).await.expect("Failed to send stream message");
        }

        for message in stream_messages {
            let next = stream_receiver.next().await;
            match message.message.unwrap() {
                model::stream_message::Message::Content(bytes) => {
                    let proposal_part =
                        model::ProposalPart::decode(bytes.as_slice()).expect("Failed to decode proposal part");
                    assert_matches::assert_matches!(next, Some(Ok(..)));
                    assert_eq!(proposal_part, next.unwrap().unwrap());
                }
                model::stream_message::Message::Fin(_) => {
                    assert!(next.is_none());
                }
            }
        }

        assert!(stream_receiver.next().await.is_none());
    }

    //     /// An accumulator should always return the number of messages it has
    //     /// received as its length
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_len(stream_message: impl Iterator<Item = model::StreamMessage>) {
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new();
    //         let mut i = 0;
    //
    //         for message in stream_message {
    //             accumulator = accumulator.accumulate(message).expect("Failed to accumulate message stream");
    //             i += 1;
    //
    //             match accumulator {
    //                 AccumulatorStateMachine::Accumulate(..) => {
    //                     assert_eq!(Some(i), accumulator.len());
    //                 }
    //                 AccumulatorStateMachine::Done { .. } => {
    //                     assert_eq!(None, accumulator.len())
    //                 }
    //             }
    //         }
    //     }
    //
    //     /// An accumulator should always correctly return its limits
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_limits(
    //         proposal_part: model::ProposalPart,
    //         stream_message: impl Iterator<Item = model::StreamMessage>,
    //     ) {
    //         let limit = proposal_part.encode_to_vec().len();
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new_with_limits(limit);
    //         let mut i = 0;
    //
    //         assert_eq!(accumulator.limits().cloned(), Some(OrderedStreamLimits { max: limit, current: 0 }));
    //
    //         for message in stream_message {
    //             accumulator = accumulator.accumulate(message).expect("Failed to accumulate message stream");
    //             i += 1;
    //         }
    //
    //         assert!(i > 1, "Proposal part was streamed over a single message");
    //         assert!(accumulator.is_done());
    //         assert_eq!(accumulator.limits(), None);
    //     }
    //
    //     /// Makes sure that incrementing a stream limit cannot result in an overflow
    //     #[test]
    //     fn ordered_stream_limits_overflow() {
    //         let mut limits = OrderedStreamLimits::new(usize::MAX);
    //         limits.update(usize::MAX).unwrap();
    //
    //         assert_eq!(limits, OrderedStreamLimits { max: usize::MAX, current: usize::MAX });
    //         assert_matches::assert_matches!(limits.update(1), Ok(()));
    //         assert_eq!(limits, OrderedStreamLimits { max: usize::MAX, current: usize::MAX });
    //     }
    //
    //     /// Limits should correctly indicate when they have been reached
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_limits_has_space_left(
    //         proposal_part: model::ProposalPart,
    //         stream_message: impl Iterator<Item = model::StreamMessage>,
    //     ) {
    //         let accumulator = AccumulatorStateMachine::<model::ProposalPart>::new_with_limits(0);
    //         assert!(!accumulator.has_space_left());
    //
    //         let limit = proposal_part.encode_to_vec().len();
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new_with_limits(limit);
    //         let mut i = 0;
    //
    //         assert!(accumulator.has_space_left());
    //
    //         for message in stream_message {
    //             accumulator = accumulator.accumulate(message).expect("Failed to accumulate message stream");
    //             i += 1;
    //         }
    //
    //         assert!(i > 1, "Proposal part was streamed over a single message");
    //         assert!(accumulator.is_done());
    //         assert!(!accumulator.has_space_left());
    //     }
    //
    //     /// An accumulator should correctly inform if it has received a FIN message.
    //     /// By convention, all accumulators which are DONE are considered to have
    //     /// received a FIN message.
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_has_fin(stream_message: impl Iterator<Item = model::StreamMessage>) {
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new();
    //         let mut i = 0;
    //
    //         assert!(!accumulator.has_fin());
    //
    //         let mut messages_rev = VecDeque::new();
    //         for message in stream_message {
    //             accumulator = accumulator.accumulate(message.clone()).expect("Failed to accumulate message stream");
    //             messages_rev.push_front(message);
    //             i += 1;
    //         }
    //
    //         assert!(i > 1, "Proposal part was streamed over a single message");
    //         assert!(accumulator.is_done());
    //         assert!(accumulator.has_fin());
    //
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new();
    //         assert!(!accumulator.has_fin());
    //
    //         accumulator =
    //             accumulator.accumulate(messages_rev.pop_front().unwrap()).expect("Failed to accumulate message stream");
    //         assert!(accumulator.has_fin());
    //
    //         for message in messages_rev {
    //             accumulator = accumulator.accumulate(message).expect("Failed to accumulate message stream");
    //         }
    //
    //         assert!(accumulator.is_done());
    //         assert!(accumulator.has_fin());
    //     }
    //
    //     /// Checks is a stream message could be the last part needed by an
    //     /// accumulator. This is a very basic check and does not perform any stream
    //     /// id or stream limit verification, so we do not test for that here either.
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_is_last_part(stream_message: impl Iterator<Item = model::StreamMessage>) {
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new();
    //
    //         let mut messages = stream_message.collect::<Vec<_>>();
    //         let fin = messages.pop().unwrap();
    //
    //         for message in messages.iter() {
    //             assert!(!accumulator.is_last_part(message));
    //             assert!(!accumulator.is_last_part(&fin));
    //             accumulator = accumulator.accumulate(message.clone()).expect("Failed to accumulate message stream");
    //         }
    //
    //         assert!(accumulator.is_last_part(&fin));
    //         accumulator = accumulator.accumulate(fin.clone()).expect("Failed to accumulate message stream");
    //         assert!(accumulator.is_done());
    //
    //         let message_last = messages.pop().unwrap();
    //         assert!(!messages.is_empty());
    //
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new();
    //         accumulator = accumulator.accumulate(fin).expect("Failed to accumulate message stream");
    //         assert!(!accumulator.is_last_part(&message_last));
    //
    //         for message in messages {
    //             assert!(!accumulator.is_last_part(&message));
    //             assert!(!accumulator.is_last_part(&message_last));
    //             accumulator = accumulator.accumulate(message).expect("Failed to accumulate message stream");
    //         }
    //
    //         assert!(accumulator.is_last_part(&message_last));
    //         accumulator = accumulator.accumulate(message_last).expect("Failed to accumulate message stream");
    //         assert!(accumulator.is_done());
    //     }
    //
    //     /// Receives a proposal part with a bound to the number of bytes which can
    //     /// be received.
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_bounded(
    //         proposal_part: model::ProposalPart,
    //         stream_message: impl Iterator<Item = model::StreamMessage>,
    //     ) {
    //         let limit = proposal_part.encode_to_vec().len();
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new_with_limits(limit);
    //         let mut i = 0;
    //
    //         for message in stream_message {
    //             accumulator = accumulator.accumulate(message).expect("Failed to accumulate message stream");
    //             i += 1;
    //         }
    //
    //         assert!(i > 1, "Proposal part was streamed over a single message");
    //         assert!(accumulator.is_done());
    //
    //         let proposal_part_actual = accumulator.consume();
    //         assert_eq!(
    //             proposal_part_actual,
    //             Some(proposal_part),
    //             "Failed to reconstruct proposal part from message stream"
    //         );
    //     }
    //
    //     /// Receives a proposal part in an _unordered_ stream. The
    //     /// [OrderedStreamAccumulator] has to sort the inputs and decode them
    //     /// correctly.
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_shuffled(
    //         proposal_part: model::ProposalPart,
    //         stream_message_shuffled: impl Iterator<Item = model::StreamMessage>,
    //     ) {
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new();
    //         let mut i = 0;
    //
    //         for message in stream_message_shuffled {
    //             accumulator = accumulator.accumulate(message).expect("Failed to accumulate message stream");
    //             i += 1;
    //         }
    //
    //         assert!(i > 1, "Proposal part was streamed over a single message");
    //         assert!(accumulator.is_done());
    //
    //         let proposal_part_actual = accumulator.consume();
    //         assert_eq!(
    //             proposal_part_actual,
    //             Some(proposal_part),
    //             "Failed to reconstruct proposal part from message stream"
    //         );
    //     }
    //
    //     /// Receives a proposal part with different stream ids. This is indicative
    //     /// of multiple streams overlapping and should not happen if the sender is
    //     /// not malfunctioning.
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_fail_invalid_stream_id(
    //         mut stream_message_invalid_stream_id: impl Iterator<Item = model::StreamMessage>,
    //     ) {
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new();
    //
    //         accumulator = accumulator
    //             .accumulate(stream_message_invalid_stream_id.next().unwrap())
    //             .expect("Failed on first message reception: this should not happen");
    //
    //         assert_matches::assert_matches!(
    //             accumulator.accumulate(stream_message_invalid_stream_id.next().unwrap()),
    //             Err(AccumulateError::InvalidStreamId(..))
    //         );
    //     }
    //
    //     /// Receives a proposal part in a stream with more bytes than is allowed in
    //     /// the stream limits.
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_fail_max_bounds(
    //         proposal_part: model::ProposalPart,
    //         stream_message: impl Iterator<Item = model::StreamMessage>,
    //     ) {
    //         let limit = proposal_part.encode_to_vec().len();
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new_with_limits(limit - 1);
    //
    //         for message in stream_message {
    //             accumulator = match accumulator.accumulate(message) {
    //                 Ok(accumulator) => accumulator,
    //                 Err(e) => {
    //                     assert_matches::assert_matches!(e, AccumulateError::MaxBounds(..));
    //                     break;
    //                 }
    //             };
    //         }
    //     }
    //
    //     /// Receives a proposal part in a stream with multiple FIN messages. This is
    //     /// considered malicious.
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_fail_double_fin(stream_message_double_fin: impl Iterator<Item = model::StreamMessage>) {
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new();
    //
    //         for message in stream_message_double_fin {
    //             accumulator = match accumulator.accumulate(message) {
    //                 Ok(accumulator) => accumulator,
    //                 Err(e) => {
    //                     assert_matches::assert_matches!(e, AccumulateError::DoubleFin(..));
    //                     break;
    //                 }
    //             };
    //         }
    //     }
    //
    //     /// Receives a proposal part in a stream. The proposal part is only
    //     /// partially sent before the FIN, so this should result in a decode error.
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_fail_decode_error(stream_message_decode_error: impl Iterator<Item = model::StreamMessage>) {
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new();
    //
    //         for message in stream_message_decode_error {
    //             accumulator = match accumulator.accumulate(message) {
    //                 Ok(accumulator) => accumulator,
    //                 Err(e) => {
    //                     assert_matches::assert_matches!(e, AccumulateError::DecodeError(..));
    //                     break;
    //                 }
    //             };
    //         }
    //     }
    //
    //     /// Receives a proposal part in a stream. Protobuf allows for all message
    //     /// fields to be optional. In our case, we consider any missing field which
    //     /// is not explicitly marked as `optional` to be required, and return an
    //     /// error if this is the case.
    //     #[rstest::rstest]
    //     #[timeout(std::time::Duration::from_secs(1))]
    //     fn ordered_stream_fail_model_error(stream_message_model_error: impl Iterator<Item = model::StreamMessage>) {
    //         let mut accumulator = AccumulatorStateMachine::<model::ProposalPart>::new();
    //
    //         for message in stream_message_model_error {
    //             accumulator = match accumulator.accumulate(message) {
    //                 Ok(accumulator) => accumulator,
    //                 Err(e) => {
    //                     assert_matches::assert_matches!(e, AccumulateError::ModelError(..));
    //                     break;
    //                 }
    //             };
    //         }
    //     }
}

// #[cfg(test)]
// mod proptest {
//     use std::collections::VecDeque;
//
//     use proptest::prelude::*;
//     use proptest::prop_compose;
//     use proptest_state_machine::ReferenceStateMachine;
//     use proptest_state_machine::StateMachineTest;
//     use prost::Message;
//     use starknet_core::types::Felt;
//
//     use crate::model;
//
//     use super::AccumulateError;
//     use super::AccumulatorStateMachine;
//
//     type SystemUnderTest = AccumulatorStateMachine<model::ProposalPart>;
//
//     proptest_state_machine::prop_state_machine! {
//         #![proptest_config(proptest::prelude::ProptestConfig {
//             // Enable verbose mode to make the state machine test print the
//             // transitions for each case.
//             verbose: 1,
//             // The number of tests which need to be valid for this to pass.
//             cases: 512,
//             // Max duration (in milliseconds) for each generated case.
//             timeout: 1_000,
//             ..Default::default()
//         })]
//
//         #[test]
//         fn ordered_stream_proptest(sequential 1..256 => SystemUnderTest);
//     }
//
//     fn stream_id() -> Vec<u8> {
//         let stream_id = model::ConsensusStreamId { height: 1, round: 1 };
//         let mut buffer = Vec::new();
//         stream_id.encode(&mut buffer).expect("Failed to encode stream id");
//
//         buffer
//     }
//
//     prop_compose! {
//         fn proposal_part()(len in 10..100usize) -> model::ProposalPart {
//             let tx = model::ConsensusTransaction {
//                 transaction_hash: Some(model::Hash(Felt::ONE)),
//                 txn: Some(model::consensus_transaction::Txn::L1Handler(model::L1HandlerV0 {
//                     nonce: Some(model::Felt252(Felt::ZERO)),
//                     address: Some(model::Address(Felt::ONE)),
//                     entry_point_selector: Some(model::Felt252(Felt::TWO)),
//                     calldata: vec![model::Felt252(Felt::THREE); 12]
//                 }))
//             };
//
//             model::ProposalPart {
//                 messages: Some(model::proposal_part::Messages::Transactions(model::TransactionBatch {
//                     transactions: vec![tx; len]
//                 }))
//             }
//         }
//     }
//
//     prop_compose! {
//         fn stream_messages(stream_id: Vec<u8>, proposal_part: model::ProposalPart)(
//             split_into in 1..256usize
//         ) -> VecDeque<model::StreamMessage> {
//             let mut buffer = Vec::new();
//             proposal_part.encode(&mut buffer).expect("Failed to encode proposal part");
//
//             buffer
//                 .chunks(buffer.len() / split_into)
//                 .map(Vec::from)
//                 .map(model::stream_message::Message::Content)
//                 .chain(std::iter::once(model::stream_message::Message::Fin(model::Fin {})))
//                 .enumerate()
//                 .map(|(i, message)| model::StreamMessage {
//                     message: Some(message),
//                     stream_id: stream_id.clone(),
//                     message_id: i as u64
//                 })
//                 .collect()
//         }
//     }
//
//     prop_compose! {
//         fn reference_state_machine()(
//             proposal_part in proposal_part()
//         )(
//             stream_messages in stream_messages(stream_id(), proposal_part.clone()),
//             proposal_part in Just(proposal_part),
//             delta in 0..10_000usize
//         ) -> OrderedStreamAccumulatorReference {
//             let size = proposal_part.encoded_len();
//             let limit = if delta > 5_000 {
//                 size.saturating_sub(delta)
//             } else {
//                 size.saturating_add(delta)
//             };
//
//             OrderedStreamAccumulatorReference {
//                 proposal_part,
//                 stream_messages,
//                 accumulator: AccumulatorStateMachine::new_with_stream_id_and_limits(
//                     stream_id().as_slice(),
//                     limit,
//                 ),
//                 error: ProptestError::None
//             }
//         }
//     }
//
//     #[derive(Clone)]
//     pub enum ProptestTransition {
//         Accumulate(model::StreamMessage),
//         ActMalicious(ProptestMaliciousTransition),
//         Consume,
//     }
//
//     #[derive(Clone)]
//     pub enum ProptestMaliciousTransition {
//         InvalidStreamId(model::StreamMessage),
//         InsertGarbageData(model::StreamMessage),
//         DoubleFin(model::StreamMessage),
//         InvalidModel(model::StreamMessage),
//     }
//
//     impl std::fmt::Debug for ProptestTransition {
//         fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//             match self {
//                 Self::Accumulate(stream_message) => match stream_message.message.as_ref().unwrap() {
//                     model::stream_message::Message::Content(..) => f.debug_tuple("Accumulate (Content)").finish(),
//                     model::stream_message::Message::Fin(..) => f.debug_tuple("Accumulate (Fin)").finish(),
//                 },
//                 Self::ActMalicious(transition) => f.debug_tuple("ActMalicious").field(&transition).finish(),
//                 Self::Consume => f.debug_tuple("Collect").finish(),
//             }
//         }
//     }
//
//     impl std::fmt::Debug for ProptestMaliciousTransition {
//         fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//             match self {
//                 Self::InvalidStreamId(..) => f.debug_tuple("InvalidStreamId").finish(),
//                 Self::InsertGarbageData(..) => f.debug_tuple("InsertGarbageData").finish(),
//                 Self::DoubleFin(..) => f.debug_tuple("DoubleFin").finish(),
//                 Self::InvalidModel(..) => f.debug_tuple("InvalidModel").finish(),
//             }
//         }
//     }
//
//     #[derive(Clone)]
//     pub struct OrderedStreamAccumulatorReference {
//         proposal_part: model::ProposalPart,
//         stream_messages: VecDeque<model::StreamMessage>,
//         accumulator: AccumulatorStateMachine<model::ProposalPart>,
//         error: ProptestError,
//     }
//
//     #[derive(Clone, Debug, PartialEq, Eq)]
//     #[repr(u8)]
//     pub enum ProptestError {
//         None = 0,
//         Decode = 1,
//         DoubleFin = 2,
//     }
//
//     impl ProptestError {
//         fn priority(&self) -> u8 {
//             self.clone() as u8
//         }
//     }
//
//     impl std::fmt::Debug for OrderedStreamAccumulatorReference {
//         fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//             let stream_messages = self
//                 .stream_messages
//                 .iter()
//                 .map(|m| {
//                     m.message.clone().map(|m| match m {
//                         model::stream_message::Message::Content(..) => "Content(...)",
//                         model::stream_message::Message::Fin(..) => "Fin",
//                     })
//                 })
//                 .take(5)
//                 .collect::<Vec<_>>();
//
//             let stream_messages = if stream_messages.len() < self.stream_messages.len() {
//                 format!("{stream_messages:?}... ({} items)", self.stream_messages.len())
//             } else {
//                 format!("{stream_messages:?} ({} items)", self.stream_messages.len())
//             };
//
//             f.debug_struct("OrderedStreamAccumulatorStateMachine")
//                 .field("stream_messages", &stream_messages)
//                 .field("error", &self.error)
//                 .finish()
//         }
//     }
//
//     impl ReferenceStateMachine for OrderedStreamAccumulatorReference {
//         type State = OrderedStreamAccumulatorReference;
//         type Transition = ProptestTransition;
//
//         fn init_state() -> BoxedStrategy<Self::State> {
//             reference_state_machine().boxed()
//         }
//
//         fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
//             if let Some(stream_message) = state.stream_messages.front() {
//                 if !state.accumulator.is_empty() {
//                     Just(ProptestTransition::Accumulate(stream_message.clone())).boxed()
//                 } else {
//                     prop_oneof![
//                         4 => Just(ProptestTransition::Accumulate(stream_message.clone())),
//                         1 => Self::act_malicious(state)
//                     ]
//                     .boxed()
//                 }
//             } else {
//                 prop_oneof! [
//                     3 => Just(ProptestTransition::Consume),
//                     1 => Self::act_malicious(state)
//                 ]
//                 .boxed()
//             }
//         }
//
//         fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
//             match transition {
//                 ProptestTransition::Accumulate(stream_message) => {
//                     state.stream_messages.pop_front();
//                     state.accumulator =
//                         state.accumulator.clone().accumulate(stream_message.clone()).unwrap_or(state.accumulator);
//                 }
//                 ProptestTransition::ActMalicious(transition) => match transition {
//                     ProptestMaliciousTransition::InsertGarbageData(stream_message) => {
//                         if state.error.priority() < ProptestError::Decode.priority() && !state.accumulator.is_done() {
//                             state.error = ProptestError::Decode;
//                         }
//
//                         state.accumulator = state
//                             .accumulator
//                             .clone()
//                             .accumulate(stream_message.clone(), true)
//                             .unwrap_or(state.accumulator);
//                     }
//                     ProptestMaliciousTransition::DoubleFin(stream_message) => {
//                         if state.error.priority() < ProptestError::DoubleFin.priority() && !state.accumulator.has_fin()
//                         {
//                             state.error = ProptestError::DoubleFin;
//                         }
//                         state.accumulator =
//                             state.accumulator.clone().accumulate(stream_message.clone()).unwrap_or(state.accumulator);
//                     }
//                     _ => (),
//                 },
//                 _ => (),
//             }
//
//             state
//         }
//     }
//
//     impl OrderedStreamAccumulatorReference {
//         fn act_malicious(state: &Self) -> impl Strategy<Value = ProptestTransition> {
//             let invalid_stream_id = || {
//                 let stream_id = state.accumulator.stream_id().unwrap();
//                 let mut stream_id = model::ConsensusStreamId::decode(stream_id).unwrap();
//                 stream_id.height += 1;
//                 stream_id.round += 1;
//
//                 let mut stream_message = state.stream_messages.front().cloned().unwrap_or_default();
//                 let mut buffer = Vec::new();
//                 stream_id.encode(&mut buffer).unwrap();
//                 stream_message.stream_id = buffer;
//
//                 ProptestMaliciousTransition::InvalidStreamId(stream_message)
//             };
//
//             let insert_garbage_data = || {
//                 let content = state
//                     .stream_messages
//                     .front()
//                     .cloned()
//                     .unwrap_or_default()
//                     .message
//                     .unwrap_or(model::stream_message::Message::Content(vec![]));
//
//                 let content = if let model::stream_message::Message::Content(mut content) = content {
//                     content.insert(0, 42);
//                     content
//                 } else {
//                     vec![42]
//                 };
//
//                 let stream_message = model::StreamMessage {
//                     message: Some(model::stream_message::Message::Content(content)),
//                     stream_id: state.accumulator.stream_id().unwrap().to_vec(),
//                     message_id: state.accumulator.len().unwrap_or(0) as u64,
//                 };
//
//                 ProptestMaliciousTransition::InsertGarbageData(stream_message)
//             };
//
//             let double_fin = || {
//                 let stream_message = model::StreamMessage {
//                     message: Some(model::stream_message::Message::Fin(model::Fin {})),
//                     stream_id: state.accumulator.stream_id().unwrap().to_vec(),
//                     message_id: u64::MAX - state.accumulator.len().unwrap_or(0) as u64,
//                 };
//
//                 ProptestMaliciousTransition::DoubleFin(stream_message)
//             };
//
//             let invalid_model = || {
//                 let stream_mesage = model::StreamMessage {
//                     message: None,
//                     stream_id: state.accumulator.stream_id().unwrap().to_vec(),
//                     message_id: u64::MAX / 2 - state.accumulator.len().unwrap_or(0) as u64,
//                 };
//
//                 ProptestMaliciousTransition::InvalidModel(stream_mesage)
//             };
//
//             prop_oneof![
//                 Just(ProptestTransition::ActMalicious(invalid_stream_id())),
//                 Just(ProptestTransition::ActMalicious(insert_garbage_data())),
//                 Just(ProptestTransition::ActMalicious(double_fin())),
//                 Just(ProptestTransition::ActMalicious(invalid_model()))
//             ]
//         }
//     }
//
//     impl StateMachineTest for AccumulatorStateMachine<model::ProposalPart> {
//         type SystemUnderTest = Self;
//         type Reference = OrderedStreamAccumulatorReference;
//
//         fn init_test(ref_state: &<Self::Reference as ReferenceStateMachine>::State) -> Self::SystemUnderTest {
//             Self::new_with_limits(ref_state.accumulator.limits().unwrap().max)
//         }
//
//         fn apply(
//             mut state: Self::SystemUnderTest,
//             ref_state: &<Self::Reference as ReferenceStateMachine>::State,
//             transition: <Self::Reference as ReferenceStateMachine>::Transition,
//         ) -> Self::SystemUnderTest {
//             match transition {
//                 ProptestTransition::Accumulate(stream_message) => {
//                     let res = state.clone().accumulate(stream_message.clone());
//
//                     if state.is_done() {
//                         assert_matches::assert_matches!(res, Ok(..));
//                         return res.unwrap();
//                     }
//
//                     match stream_message.message {
//                         Some(model::stream_message::Message::Content(..)) => {
//                             if !Self::check_limits(&res, stream_message.clone(), &state) {
//                                 assert_matches::assert_matches!(
//                                     res,
//                                     Ok(..),
//                                     "Accumulate error with valid bounds: {state:?}"
//                                 );
//                             }
//                         }
//                         Some(model::stream_message::Message::Fin(..)) => {
//                             if ProptestError::None == ref_state.error {
//                                 assert_matches::assert_matches!(
//                                     res,
//                                     Ok(..),
//                                     "Accumulate error when none expected: {state:?}"
//                                 )
//                             }
//                         }
//                         _ => (),
//                     }
//
//                     Self::check_last_part(&res, &stream_message, &state, ref_state);
//
//                     // We always set the stream id even in the case of an error.
//                     // This insures that subsequent invalid streams ids are seen
//                     // as such.
//                     if let AccumulatorStateMachine::Accumulate(mut inner) = state {
//                         inner.stream_id = Some(stream_message.stream_id);
//                         state = AccumulatorStateMachine::Accumulate(inner);
//                     }
//
//                     res.unwrap_or(state)
//                 }
//                 ProptestTransition::ActMalicious(transition) => match transition {
//                     ProptestMaliciousTransition::InvalidStreamId(stream_message) => {
//                         let res = state.clone().accumulate(stream_message.clone());
//                         assert_matches::assert_matches!(res, Err(AccumulateError::InvalidStreamId(..)));
//                         state
//                     }
//                     ProptestMaliciousTransition::InsertGarbageData(stream_message) => {
//                         let res = state.clone().accumulate(stream_message.clone(), true);
//
//                         if !Self::check_last_part(&res, &stream_message, &state, ref_state) {
//                             assert_matches::assert_matches!(res, Ok(..));
//                         }
//
//                         res.unwrap_or(state)
//                     }
//                     ProptestMaliciousTransition::DoubleFin(stream_message) => {
//                         let res = state.clone().accumulate(stream_message.clone(), true);
//
//                         if state.is_done() {
//                             assert_matches::assert_matches!(res, Ok(..));
//                             return res.unwrap();
//                         } else if state.has_fin() {
//                             assert_matches::assert_matches!(res, Err(AccumulateError::DoubleFin(..)));
//                         }
//
//                         res.unwrap_or(state)
//                     }
//                     ProptestMaliciousTransition::InvalidModel(stream_message) => {
//                         let res = state.clone().accumulate(stream_message.clone(), true);
//
//                         if state.is_done() {
//                             assert_matches::assert_matches!(res, Ok(..));
//                             return res.unwrap();
//                         }
//
//                         assert_matches::assert_matches!(res, Err(AccumulateError::ModelError(..)));
//                         state
//                     }
//                 },
//                 ProptestTransition::Consume => {
//                     let res = state.clone().consume();
//                     if state.is_done() && ref_state.error == ProptestError::None {
//                         assert!(res.is_some(), "Complete stream returned None: {state:?}");
//                         assert_eq!(
//                             res,
//                             Some(ref_state.proposal_part.clone()),
//                             "Collected stream does not match: {state:?}"
//                         );
//                     } else {
//                         assert!(res.is_none(), "Incomplete stream returned Some: {state:?}")
//                     }
//                     state
//                 }
//             }
//         }
//     }
//
//     impl AccumulatorStateMachine<model::ProposalPart> {
//         fn check_limits(
//             res: &Result<Self, AccumulateError>,
//             stream_message: model::StreamMessage,
//             state: &Self,
//         ) -> bool {
//             if let AccumulatorStateMachine::Accumulate(inner) = &state {
//                 if let model::stream_message::Message::Content(bytes) = stream_message.message.clone().unwrap() {
//                     if inner.limits.clone().update(bytes.len()).is_err() {
//                         assert_matches::assert_matches!(res, Err(AccumulateError::MaxBounds(..)));
//                         return true;
//                     }
//                 }
//             }
//             false
//         }
//
//         fn check_last_part(
//             res: &Result<Self, AccumulateError>,
//             stream_message: &model::StreamMessage,
//             state: &Self,
//             ref_state: &OrderedStreamAccumulatorReference,
//         ) -> bool {
//             if state.is_last_part(stream_message) {
//                 match ref_state.error {
//                     ProptestError::None => {
//                         assert_matches::assert_matches!(
//                             res,
//                             Ok(AccumulatorStateMachine::Done { .. }),
//                             "Accumulator is not Done after receiving Fin"
//                         )
//                     }
//                     ProptestError::Decode => {
//                         assert_matches::assert_matches!(res, Err(AccumulateError::DecodeError(..)))
//                     }
//                     ProptestError::DoubleFin => {
//                         assert_matches::assert_matches!(res, Err(AccumulateError::DoubleFin(..)))
//                     }
//                 }
//                 true
//             } else {
//                 false
//             }
//         }
//     }
// }

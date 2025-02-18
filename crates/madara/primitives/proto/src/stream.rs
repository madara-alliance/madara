use std::{
    collections::{btree_map, BTreeMap},
    task::Poll,
};

use m_proc_macros::model_describe;

use crate::{
    model::{self, stream_message},
    model_field,
};

/// A list of potential errors which can arise when processing an [OrderedMessageStream].
///
/// > Note that the occurrence of any error during the execution or polling of
/// > [OrderedMessageStream] should be regarded as indicating malicious behavior.
#[derive(thiserror::Error, Debug)]
pub enum StreamReceiverError<StreamId>
where
    StreamId: std::fmt::Debug,
{
    /// A message was received with a stream id which does not match the current stream.
    #[error("Invalid stream id: expected {1:?}, got {0:?}")]
    InvalidStreamId(StreamId, StreamId),
    /// A message was sent with a message id greater than or equal to the message id of [Fin].
    ///
    /// [Fin]: model::stream_message::Message::Fin
    #[error("Already received a Fin before this message")]
    SendAfterFin,
    /// A second [Fin] message was received after the stream had already processed one.
    ///
    /// [Fin]: model::stream_message::Message::Fin
    #[error("New Fin with id {0} but already received Fin at message id {1}")]
    DoubleFin(u64, u64),
    /// A message with the same message id was sent twice to the same stream.
    #[error("Message with id {0} has already been received")]
    DoubleSend(u64),
    /// A [Content] message was sent with a payload which could not be decoded.
    ///
    /// [Content]: model::stream_message::Message::Content
    #[error("Failed to decode model: {0}")]
    DecodeError(#[from] prost::DecodeError),
    /// A message was sent with neither [Fin] nor [Content].
    ///
    /// [Fin]: model::stream_message::Message::Fin
    /// [Content]: model::stream_message::Message::Content
    #[error(transparent)]
    ModelError(#[from] crate::FromModelError),
    /// The [tokio::sync::mpsc] channel receiver was closed but the stream is still listening.
    ///
    /// > This should not happen in any sane environment.
    #[error("Channel was closed over an unfinished stream")]
    ChannelError,
    /// A message was sent with a message id further from all other messages by more than the
    /// stream's lag cap.
    #[error("The stream has lagged too far behind")]
    LagCap,
}

// TODO: remove this
pub type ConsensusStreamBuilder = StreamBuilder<model::ProposalPart, model::ConsensusStreamId>;

/// Messaging stream for use in P2P consensus.
///
/// See [OrderedMessageStream] for more information.
pub type ConsensusStream = OrderedMessageStream<model::ProposalPart, model::ConsensusStreamId>;

/// Ordered stream item for use in P2P consensus.
///
/// See [StreamItem] for more information.
pub type ConsensusStreamItem = StreamItem<model::ProposalPart, model::ConsensusStreamId>;

/// An asyncronous messaging stream.
///
/// `OrderedMessageStream` is a generic construct for receiving streamed messages via the
/// [StreamMessage] protobuf transport. These messages can be received out-of-order and
/// `OrderedMessageStream` will yield them in-order in a non-blocking way.
///
/// # Ordering
///
/// Messages are streamed over a [tokio::sync::mpsc] channel, with `OrderedMessageStream` holding
/// the receiving end. This channel is polled on each call to [poll_next] until the next ordered
/// message is found. Otherwise, the stream yields execution back to the async monitor.
///
/// # Streaming process
///
/// A streamed message can hold either a [Content] or [Fin]. A [Content] message holds the data
/// being streamed. A [Fin] message marks the end of a stream. Both [Content] and [Fin] messages are
/// marked by a `message_id`, which denotes their order in the stream. `message_id` is incremental
/// and should start at 0. Keep in mind that messages can be received out-of-order, so [Fin] could
/// be received before the messages which precede it! This is taken care of in the ordering logic.
/// Messages are also marked with a `stream_id` which prevents messages from two different streams
/// from getting mixed up. Ordered messages are polled as a [StreamItem] containing the message
/// itself as well as some additional data.
///
/// > You might notice that [Content] is made generic over any transport by storing its payload as
/// > raw bytes. We enforce that this payload must be able to be decoded based on the `Message`
/// > generic of `OrderedMessageStream`. Failure to do so is an error and will be considered as
/// > malicious.
///
/// # State machine
///
/// `OrderedMessageStream` represents its inner state as a state machine:
///
/// - [AccumulatorStateMachine::Accumulate]: the stream has not yet yielded all ordered messages.
///   It is possible to have already received a [Fin] in this state but still not have processed all
///   the messages before it.
///
/// - [AccumulatorStateMachine::Done]: the stream has finished execution and will no longer yield
///   any messages. If it is polled, it will simply return [Poll::Ready(None)]. A stream will also
///   enter this state and auto-close its channel if ever an [error] is encountered.
///
/// # DOS mitigation
///
/// Basic DOS mitigation is in place to avoid the stream hanging indefinitely. This is implemented
/// as a _lag cap_. At any point in the state of the stream, messages cannot have their message ids
/// be apart by more than the lag cap. If a message is received which has its id apart by more than
/// that, then the stream will return an error. This is considered to be malicious.
///
/// Notice that there is no hard cap on the maximum number of messages which can be sent, so it is
/// up to the caller to implement more advanced strategies such as rate limiting and timeout
/// policies.
///
/// [StreamMessage]: model::StreamMessage
/// [poll_next]: futures::Stream::poll_next
/// [Content]: model::stream_message::Message::Content
/// [Fin]: model::stream_message::Message::Fin
/// [Poll::Ready(None)]: std::task::Poll::Ready(None)
/// [eror]: StreamReceiverError
pub struct OrderedMessageStream<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    receiver: tokio::sync::mpsc::Receiver<model::StreamMessage>,
    state: AccumulatorStateMachine<Message, StreamId>,
}

/// An ordered message returned by a [OrderedMessageStream].
///
/// This struct is a wrapper around the message itself, along with some extra information such as
/// the message id and the id of the stream which produced the message.
#[derive(Clone, Debug)]
pub struct StreamItem<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    pub message: Message,
    pub id_message: u64,
    pub id_stream: StreamId,
}

// TODO: remove this. This is just legacy from when the stream id used to be optional.
/// A type safe builder for [OrderedMessageStream].
pub struct StreamBuilder<Message, StreamId, T = Never, U = Never, V = Never>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    lag_cap: T,
    channel_cap: U,
    id_stream: V,
    _phantom1: std::marker::PhantomData<Message>,
    _phantom2: std::marker::PhantomData<StreamId>,
}

#[derive(Debug)]
#[cfg_attr(test, derive(Clone))]
enum AccumulatorStateMachine<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    Accumulate(AccumulatorStateInner<Message, StreamId>),
    Done,
}

#[cfg_attr(test, derive(Clone))]
struct AccumulatorStateInner<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    messages: BTreeMap<u64, Message>,
    id_stream_decode: StreamId,
    id_stream: Vec<u8>,
    id_low: u64,
    fin: Option<u64>,
    lag_cap: u64,
    _phantom: std::marker::PhantomData<StreamId>,
}

impl Eq for model::ConsensusStreamId {}

#[cfg(not(test))]
impl<Message, StreamId> PartialEq for StreamItem<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.id_stream == other.id_stream && self.id_message == other.id_message
    }
}

#[cfg(not(test))]
impl<Message, StreamId> Eq for StreamItem<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
}

#[cfg(not(test))]
impl<Message, StreamId> Ord for StreamItem<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id_message.cmp(&other.id_message)
    }
}

#[cfg(not(test))]
impl<Message, StreamId> PartialOrd for StreamItem<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
impl PartialEq for ConsensusStreamItem {
    fn eq(&self, other: &Self) -> bool {
        self.id_stream == other.id_stream && self.id_message == other.id_message && self.message == other.message
    }
}

#[cfg(test)]
impl Eq for ConsensusStreamItem {}

#[cfg(test)]
impl Ord for ConsensusStreamItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id_message.cmp(&other.id_message)
    }
}

#[cfg(test)]
impl PartialOrd for ConsensusStreamItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<Message, StreamId> std::fmt::Debug for OrderedMessageStream<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
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
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccumulatorStateInner")
            .field("messages", &format!("... ({})", self.messages.len()))
            .field("stream_id", &self.id_stream_decode)
            .field("id_low", &self.id_low)
            .field("lag_cap", &self.lag_cap)
            .field("fin", &self.fin)
            .finish()
    }
}

type Never = Option<std::convert::Infallible>;

impl<T, U, V, Message, StreamId> StreamBuilder<Message, StreamId, T, U, V>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    pub fn new() -> StreamBuilder<Message, StreamId, Never, Never, Never> {
        StreamBuilder {
            lag_cap: None,
            channel_cap: None,
            id_stream: None,
            _phantom1: std::marker::PhantomData,
            _phantom2: std::marker::PhantomData,
        }
    }

    pub fn with_lag_cap(self, lag_cap: u64) -> StreamBuilder<Message, StreamId, u64, U, V> {
        let Self { channel_cap, id_stream: stream_id, _phantom1, _phantom2, .. } = self;
        StreamBuilder { lag_cap, channel_cap, id_stream: stream_id, _phantom1, _phantom2 }
    }

    pub fn with_channel_cap(self, channel_cap: usize) -> StreamBuilder<Message, StreamId, T, usize, V> {
        let Self { lag_cap, id_stream: stream_id, _phantom1, _phantom2, .. } = self;
        StreamBuilder { lag_cap, channel_cap, id_stream: stream_id, _phantom1, _phantom2 }
    }

    pub fn with_id_stream(self, id_stream: &[u8]) -> StreamBuilder<Message, StreamId, T, U, Vec<u8>> {
        let Self { lag_cap, channel_cap, _phantom1, _phantom2, .. } = self;
        StreamBuilder { lag_cap, channel_cap, id_stream: id_stream.to_vec(), _phantom1, _phantom2 }
    }
}

impl<Message, StreamId> StreamBuilder<Message, StreamId, u64, usize, Vec<u8>>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    #[allow(clippy::type_complexity)]
    pub fn build(
        self,
    ) -> Result<
        (tokio::sync::mpsc::Sender<model::StreamMessage>, OrderedMessageStream<Message, StreamId>),
        StreamReceiverError<StreamId>,
    > {
        let Self { lag_cap, channel_cap, id_stream, .. } = self;
        let (sx, rx) = tokio::sync::mpsc::channel(channel_cap);
        let state = AccumulatorStateMachine::new(lag_cap, &id_stream)?;

        let receiver = OrderedMessageStream { receiver: rx, state };

        Ok((sx, receiver))
    }
}

impl<Message, StreamId> futures::Stream for OrderedMessageStream<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    type Item = Result<StreamItem<Message, StreamId>, StreamReceiverError<StreamId>>;

    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Option<Self::Item>> {
        let Self { receiver, state: AccumulatorStateMachine::Accumulate(inner) } = self.as_mut().get_mut() else {
            return Poll::Ready(None);
        };

        loop {
            if let Some(item) = inner.next_ordered() {
                return Poll::Ready(Some(Ok(item)));
            }

            if inner.is_done() {
                return self.done(Poll::Ready(None));
            }

            let stream_message = match std::task::ready!(receiver.poll_recv(cx)) {
                Some(stream_message) => stream_message,
                None => return self.done(Poll::Ready(Some(Err(StreamReceiverError::ChannelError)))),
            };

            if let Err(e) = inner.accumulate(stream_message) {
                return self.done(Poll::Ready(Some(Err(e))));
            };
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

impl<Message, StreamId> OrderedMessageStream<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    fn done<T>(&mut self, t: T) -> T {
        self.receiver.close();
        self.state = AccumulatorStateMachine::Done;
        t
    }
}

impl<Message, StreamId> AccumulatorStateMachine<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    fn new(lag_cap: u64, id_stream: &[u8]) -> Result<Self, StreamReceiverError<StreamId>> {
        Ok(Self::Accumulate(AccumulatorStateInner::new(id_stream, lag_cap)?))
    }
}

impl<Message, StreamId> AccumulatorStateInner<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + std::fmt::Debug + Default + Clone + Eq + PartialEq,
{
    fn new(id_stream: &[u8], lag_cap: u64) -> Result<Self, StreamReceiverError<StreamId>> {
        Ok(Self {
            messages: Default::default(),
            id_stream_decode: StreamId::decode(id_stream)?,
            id_stream: id_stream.to_vec(),
            fin: None,
            id_low: 0,
            lag_cap,
            _phantom: std::marker::PhantomData,
        })
    }

    #[model_describe(model::StreamMessage)]
    // TODO: this needs some more care in regards to DOS migitation:
    //  - limits on the total number of messsages
    fn accumulate(&mut self, stream_message: model::StreamMessage) -> Result<(), StreamReceiverError<StreamId>> {
        let id_first = self.messages.first_key_value().map(|(k, _)| *k);
        let id_last = self.messages.last_key_value().map(|(k, _)| *k);
        let id_message = stream_message.message_id;

        Self::check_stream_id(&stream_message.stream_id, &self.id_stream)?;
        Self::check_id_message(stream_message.message_id, self.id_low, self.fin)?;
        Self::check_lag_cap(id_message, id_first, id_last, self.fin, self.lag_cap)?;

        match model_field!(stream_message => message) {
            stream_message::Message::Content(bytes) => self.update_content(id_message, &bytes),
            stream_message::Message::Fin(..) => self.update_fin(id_message),
        }
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

    fn check_id_message(id_message: u64, id_low: u64, fin: Option<u64>) -> Result<(), StreamReceiverError<StreamId>> {
        if id_message < id_low {
            return Err(StreamReceiverError::DoubleSend(id_message));
        }

        if let Some(fin) = fin {
            if id_message >= fin {
                return Err(StreamReceiverError::SendAfterFin);
            }
        }
        Ok(())
    }

    fn check_lag_cap(
        id_message: u64,
        id_first: Option<u64>,
        id_last: Option<u64>,
        fin: Option<u64>,
        lag_cap: u64,
    ) -> Result<(), StreamReceiverError<StreamId>> {
        let lag = |id: Option<u64>| id.map(|id| id_message.abs_diff(id)).unwrap_or(lag_cap);
        if *[lag(id_first), lag(id_last), lag(fin)].iter().max().unwrap() > lag_cap {
            return Err(StreamReceiverError::LagCap);
        }

        Ok(())
    }

    fn update_content(&mut self, id_message: u64, bytes: &[u8]) -> Result<(), StreamReceiverError<StreamId>> {
        match self.messages.entry(id_message) {
            btree_map::Entry::Vacant(entry) => entry.insert(Message::decode(bytes)?),
            btree_map::Entry::Occupied(_) => return Err(StreamReceiverError::DoubleSend(id_message)),
        };

        Ok(())
    }

    fn update_fin(&mut self, fin_id: u64) -> Result<(), StreamReceiverError<StreamId>> {
        self.fin = match self.fin {
            Some(fin_id_old) => return Err(StreamReceiverError::DoubleFin(fin_id, fin_id_old)),
            None => Some(fin_id),
        };

        if let Some((last_id, _)) = self.messages.last_key_value() {
            if *last_id >= fin_id {
                return Err(StreamReceiverError::SendAfterFin);
            }
        }

        Ok(())
    }

    fn next_ordered(&mut self) -> Option<StreamItem<Message, StreamId>> {
        self.messages.first_entry().and_then(|entry| {
            let id_message = *entry.key();

            if id_message == self.id_low {
                self.id_low += 1;
                let message = entry.remove();
                let id_stream = self.id_stream_decode.clone();
                Some(StreamItem { message, id_message, id_stream })
            } else {
                None
            }
        })
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
mod fixtures {

    use prost::Message;
    use starknet_core::types::Felt;

    use crate::model::{self};

    use super::ConsensusStreamItem;

    #[allow(unused)]
    const STREAM_LEN_DEFAULT: u32 = 10;

    pub(crate) fn model_encode<M>(proposal_part: M) -> Vec<u8>
    where
        M: prost::Message + Default,
    {
        let mut buffer = Vec::with_capacity(proposal_part.encoded_len());
        proposal_part.encode(&mut buffer).expect("Failed to encode model");

        buffer
    }

    pub(crate) fn stream_item(bytes: &[u8], id_message: u64, id_stream: &[u8]) -> ConsensusStreamItem {
        ConsensusStreamItem {
            message: model::ProposalPart::decode(bytes).expect("Failed to decode proposal part"),
            id_message,
            id_stream: model::ConsensusStreamId::decode(id_stream).expect("Failed to decode stream id"),
        }
    }

    #[rstest::fixture]
    pub(crate) fn proposal_part(#[default(0)] seed: u32) -> model::ProposalPart {
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
    pub(crate) fn stream_proposal_part(#[default(1)] len: u32) -> impl Iterator<Item = model::stream_message::Message> {
        (0..len)
            .map(proposal_part)
            .map(model_encode)
            .map(model::stream_message::Message::Content)
            .chain(std::iter::once(model::stream_message::Message::Fin(model::Fin {})))
    }

    #[rstest::fixture]
    pub(crate) fn stream_id(#[default(0)] seed: u32) -> Vec<u8> {
        let stream_id = model::ConsensusStreamId { height: seed as u64, round: seed + 1 };
        model_encode(stream_id)
    }

    #[rstest::fixture]
    pub(crate) fn stream_messages(
        #[allow(unused)]
        #[default(STREAM_LEN_DEFAULT)]
        stream_len: u32,
        #[with(stream_len)] stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
        #[with(1)] stream_id: Vec<u8>,
    ) -> impl Iterator<Item = model::StreamMessage> {
        stream_proposal_part.enumerate().map(move |(i, message)| model::StreamMessage {
            message: Some(message),
            stream_id: stream_id.clone(),
            message_id: i as u64,
        })
    }

    #[rstest::fixture]
    pub(crate) fn stream_messages_rev(
        #[allow(unused)]
        #[default(STREAM_LEN_DEFAULT)]
        stream_len: u32,
        #[with(stream_len)] stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) -> impl Iterator<Item = model::StreamMessage> {
        stream_messages.collect::<Vec<_>>().into_iter().rev()
    }
}

#[cfg(test)]
mod test {
    use futures::{Stream, StreamExt};
    use prost::Message;

    use crate::{
        model::{self},
        stream::{fixtures::*, ConsensusStreamBuilder, StreamReceiverError},
    };

    /// Receives a proposal part in a single, ordered stream. All should work as
    /// expected
    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_simple(
        #[with(1)] stream_id: Vec<u8>,
        #[with(10)] stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, mut stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        for message in stream_messages {
            stream_sender.send(message.clone()).await.expect("Failed to send stream message");
            let next = stream_receiver.next().await;

            match message.message.unwrap() {
                model::stream_message::Message::Content(bytes) => {
                    assert_matches::assert_matches!(next, Some(Ok(..)));
                    assert_eq!(stream_item(&bytes, message.message_id, &message.stream_id), next.unwrap().unwrap());
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
        #[with(1)] stream_id: Vec<u8>,
        #[with(10)] stream_messages: impl Iterator<Item = model::StreamMessage>,
        #[with(10)] stream_messages_rev: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, mut stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        for message in stream_messages_rev {
            stream_sender.send(message).await.expect("Failed to send stream message");
        }

        for message in stream_messages {
            let next = stream_receiver.next().await;
            match message.message.unwrap() {
                model::stream_message::Message::Content(bytes) => {
                    assert_matches::assert_matches!(next, Some(Ok(..)));
                    assert_eq!(stream_item(&bytes, message.message_id, &message.stream_id), next.unwrap().unwrap());
                }
                model::stream_message::Message::Fin(_) => {
                    assert!(next.is_none());
                }
            }
        }

        assert!(stream_receiver.next().await.is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_pending(
        #[with(1)] stream_id: Vec<u8>,
        #[with(10)] stream_messages: impl Iterator<Item = model::StreamMessage>,
        #[with(10)] stream_messages_rev: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);
        for stream_message in stream_messages_rev {
            let id_message = stream_message.message_id;
            stream_sender.send(stream_message).await.expect("Failed to send stream message");

            if id_message != 0 {
                assert_matches::assert_matches!(fut.enter(|cx, rx| rx.poll_next(cx)), std::task::Poll::Pending);
            } else {
                assert!(fut.is_woken());
            }
        }

        for message in stream_messages {
            if let model::stream_message::Message::Content(bytes) = message.message.unwrap() {
                assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(si))) => {
                   assert_eq!(si, stream_item(&bytes, message.message_id, &message.stream_id));
                });
            } else {
                assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None))
            }
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_wake(
        #[with(1)] stream_id: Vec<u8>,
        #[with(2)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let message_first = stream_messages.next().unwrap();
        let message_second = stream_messages.next().unwrap();
        let fin = stream_messages.next().unwrap();

        let message = message_first.message.clone().unwrap();
        let model::stream_message::Message::Content(bytes) = message else { unreachable!() };
        let stream_item_first = stream_item(&bytes, message_first.message_id, &message_first.stream_id);

        let message = message_second.message.clone().unwrap();
        let model::stream_message::Message::Content(bytes) = message else { unreachable!() };
        let stream_item_second = stream_item(&bytes, message_second.message_id, &message_second.stream_id);

        stream_sender.send(message_second).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(fut.enter(|cx, rx| rx.poll_next(cx)), std::task::Poll::Pending);

        stream_sender.send(message_first).await.expect("Failed to send stream message");
        assert!(fut.is_woken());

        stream_sender.send(fin).await.expect("Failed to send stream message");

        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(si))) => {
            assert_eq!(si, stream_item_first);
        });
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(si))) => {
            assert_eq!(si, stream_item_second);
        });
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_lag(
        #[with(1)] stream_id: Vec<u8>,
        #[with(5)] mut stream_messages_rev: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(3)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);

        for message in stream_messages_rev.by_ref().take(4) {
            stream_sender.send(message).await.expect("Failed to send stream message");
            assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Pending);
        }

        let lag = stream_messages_rev.next().unwrap();
        stream_sender.send(lag).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(
            fut.poll_next(),
            std::task::Poll::Ready(Some(Err(StreamReceiverError::LagCap)))
        );
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_channel_close(
        #[with(1)] stream_id: Vec<u8>,
        #[with(1)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let message = stream_messages.next().unwrap();
        let model::stream_message::Message::Content(bytes) = message.message.clone().unwrap() else { unreachable!() };
        let stream_item = stream_item(&bytes, message.message_id, &message.stream_id);

        stream_sender.send(message).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(si))) => {
            assert_eq!(si, stream_item);
        });

        // In any sane environment this should not be possible
        fut.enter(|_, mut rx| rx.receiver.close());
        assert_matches::assert_matches!(
            fut.poll_next(),
            std::task::Poll::Ready(Some(Err(StreamReceiverError::ChannelError)))
        );
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_done(
        #[with(1)] stream_id: Vec<u8>,
        #[with(0)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);
        let fin = stream_messages.next().unwrap();

        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Pending);
        stream_sender.send(fin).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_invalid_stream_id_1(
        #[with(1)] stream_id: Vec<u8>,
        #[with(2)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let first = stream_messages.next().unwrap();
        let model::stream_message::Message::Content(bytes) = first.message.clone().unwrap() else { unreachable!() };
        let stream_item = stream_item(&bytes, first.message_id, &first.stream_id);

        stream_sender.send(first).await.expect("Failed to send proposal part");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(si))) => {
            assert_eq!(si, stream_item);
        });

        let mut second = stream_messages.next().unwrap();
        let bytes = second.stream_id.as_slice();
        let mut stream_id = model::ConsensusStreamId::decode(bytes).expect("Failed to decode stream id");
        let stream_id_expected = stream_id;

        stream_id.round += 1;
        stream_id.height += 1;

        second.stream_id = model_encode(stream_id);

        stream_sender.send(second).await.expect("Failed to send proposal part");
        assert_matches::assert_matches!(
            fut.poll_next(),
            std::task::Poll::Ready(Some(Err(StreamReceiverError::InvalidStreamId(s1, s2)))) => {
                assert_eq!(s1, stream_id);
                assert_eq!(s2, stream_id_expected);
            }
        );
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_invalid_stream_id_2(
        #[with(1)] stream_id: Vec<u8>,
        #[with(2)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let first = stream_messages.next().unwrap();
        let model::stream_message::Message::Content(bytes) = first.message.clone().unwrap() else { unreachable!() };
        let stream_item = stream_item(&bytes, first.message_id, &first.stream_id);

        stream_sender.send(first).await.expect("Failed to send proposal part");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(si))) => {
            assert_eq!(si, stream_item);
        });

        let mut second = stream_messages.next().unwrap();
        let bytes = second.stream_id.as_slice();
        let stream_id = model::ConsensusStreamId::decode(bytes).expect("Failed to decode stream id");

        let mut bytes = model_encode(stream_id);
        bytes.push(1);
        second.stream_id = bytes;

        stream_sender.send(second).await.expect("Failed to send proposal part");
        assert_matches::assert_matches!(
            fut.poll_next(),
            std::task::Poll::Ready(Some(Err(StreamReceiverError::DecodeError(_))))
        );
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_send_after_fin_1(
        #[with(1)] stream_id: Vec<u8>,
        #[with(1)] mut stream_messages_rev: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let fin = stream_messages_rev.next().unwrap();
        stream_sender.send(fin).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Pending);

        let mut send_after_fin = stream_messages_rev.next().unwrap();
        send_after_fin.message_id = u64::MAX;

        stream_sender.send(send_after_fin).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(
            fut.poll_next(),
            std::task::Poll::Ready(Some(Err(StreamReceiverError::SendAfterFin)))
        );
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_send_after_fin_2(
        #[with(1)] stream_id: Vec<u8>,
        #[with(1)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let mut send_after_fin = stream_messages.next().unwrap();
        send_after_fin.message_id = 2;

        stream_sender.send(send_after_fin).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Pending);

        let fin = stream_messages.next().unwrap();
        stream_sender.send(fin).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(
            fut.poll_next(),
            std::task::Poll::Ready(Some(Err(StreamReceiverError::SendAfterFin)))
        );
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_double_fin(
        #[with(1)] stream_id: Vec<u8>,
        #[from(stream_messages)]
        #[with(0)]
        mut fin: impl Iterator<Item = model::StreamMessage>,
        #[from(stream_messages)]
        #[with(0)]
        mut double_fin: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let mut fin = fin.next().unwrap();
        fin.message_id = u64::MAX;
        stream_sender.send(fin).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Pending);

        let double_fin = double_fin.next().unwrap();
        stream_sender.send(double_fin).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(
            fut.poll_next(),
            std::task::Poll::Ready(Some(Err(StreamReceiverError::DoubleFin(f1, f2)))) => {
                assert_eq!(f1, 0);
                assert_eq!(f2, u64::MAX);
            }
        );
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_double_send_1(
        #[with(1)] stream_id: Vec<u8>,
        #[with(2)] mut stream_messages_rev: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);
        let _fin = stream_messages_rev.next().unwrap();

        let first = stream_messages_rev.next().unwrap();
        stream_sender.send(first).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Pending);

        let mut double_send = stream_messages_rev.next().unwrap();
        double_send.message_id = 1;

        stream_sender.send(double_send).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(
            fut.poll_next(),
            std::task::Poll::Ready(Some(Err(StreamReceiverError::DoubleSend(id)))) => {
                assert_eq!(id, 1);
            }
        );
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_double_send_2(
        #[with(1)] stream_id: Vec<u8>,
        #[with(2)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let first = stream_messages.next().unwrap();
        let model::stream_message::Message::Content(bytes) = first.message.clone().unwrap() else { unreachable!() };
        let stream_item = stream_item(&bytes, first.message_id, &first.stream_id);

        stream_sender.send(first).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(si))) => {
            assert_eq!(si, stream_item);
        });

        let mut double_send = stream_messages.next().unwrap();
        double_send.message_id = 0;

        stream_sender.send(double_send).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(
            fut.poll_next(),
            std::task::Poll::Ready(Some(Err(StreamReceiverError::DoubleSend(id)))) => {
                assert_eq!(id, 0);
            }
        );
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_decode_error(
        #[with(1)] stream_id: Vec<u8>,
        #[with(1)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let mut decode_error = stream_messages.next().unwrap();
        let Some(model::stream_message::Message::Content(ref mut bytes)) = decode_error.message else { unreachable!() };
        bytes.push(1);

        stream_sender.send(decode_error).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(
            fut.poll_next(),
            std::task::Poll::Ready(Some(Err(StreamReceiverError::DecodeError(_))))
        );
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_model_error(
        #[with(1)] stream_id: Vec<u8>,
        #[with(1)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (stream_sender, stream_receiver) = ConsensusStreamBuilder::new()
            .with_lag_cap(u64::MAX)
            .with_channel_cap(1_000)
            .with_id_stream(&stream_id)
            .build()
            .expect("Failed to create consensus stream");

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let mut model_error = stream_messages.next().unwrap();
        model_error.message = None;

        stream_sender.send(model_error).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(
            fut.poll_next(),
            std::task::Poll::Ready(Some(Err(StreamReceiverError::ModelError(_))))
        );
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }
}

#[cfg(test)]
mod proptest {
    use std::collections::BTreeMap;
    use std::collections::VecDeque;

    use proptest::prelude::*;
    use proptest::prop_compose;
    use proptest_state_machine::ReferenceStateMachine;
    use proptest_state_machine::StateMachineTest;
    use prost::Message;

    use crate::{model, stream::fixtures};

    use super::fixtures::stream_item;
    use super::AccumulatorStateMachine;
    use super::ConsensusStream;
    use super::ConsensusStreamBuilder;
    use super::ConsensusStreamItem;

    struct SystemUnderTest {
        stream_sender: tokio::sync::mpsc::Sender<model::StreamMessage>,
        stream_receiver: ConsensusStream,
        messages_processed: Vec<ConsensusStreamItem>,
    }

    proptest_state_machine::prop_state_machine! {
        #![proptest_config(proptest::prelude::ProptestConfig {
            // Enable verbose mode to make the state machine test print the
            // transitions for each case.
            verbose: 1,
            // The number of tests which need to be valid for this to pass.
            cases: 256,
            // Max duration (in milliseconds) for each generated case.
            timeout: 1_000,
            ..Default::default()
        })]

        /// Simulates the execution of an [OrderedMessageStream] over many random and unordedered
        /// sequences of [StreamMessage]s.
        ///
        /// The max transition count of 160 is chosen so that streams will be polled beyond
        /// completion approximately 1/5th of the time (streams have a max length of 128 = 4/5*160).
        ///
        /// > Note that [proptest_state_machine] will automatically shuffle the transitions we give
        /// > it so there is no need to do that ourselves.
        ///
        /// [StreamMessage]: model::StreamMessage
        /// [OrderedMessageStream]: super::OrderedMessageStream
        #[test]
        fn ordered_stream_proptest(sequential 1..160 => SystemUnderTest);
    }

    prop_compose! {
        fn stream_messages()(len in 1..128u32) -> VecDeque<model::StreamMessage> {
            let proposals = fixtures::stream_proposal_part(len);
            let stream_id = fixtures::stream_id(1);
            let messages = fixtures::stream_messages(len, proposals, stream_id);

            return messages.collect()
        }
    }

    prop_compose! {
        fn reference_state_machine()(
            messages_to_send in stream_messages(),
        ) -> OrderedStreamReference {
            OrderedStreamReference {
                messages_to_send,
                messages_processed: Default::default(),
                id_stream: fixtures::stream_id(1),
                lag_cap: 1_000,
                fin: None,
            }
        }
    }

    #[derive(Clone)]
    struct OrderedStreamReference {
        messages_to_send: VecDeque<model::StreamMessage>,
        messages_processed: BTreeMap<u64, model::StreamMessage>,
        id_stream: Vec<u8>,
        lag_cap: u64,
        fin: Option<u64>,
    }

    #[derive(Clone)]
    enum OrderedStreamTransition {
        Send(model::StreamMessage),
        Receive,
    }

    impl std::fmt::Debug for OrderedStreamReference {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let stream_id =
                model::ConsensusStreamId::decode(self.id_stream.as_slice()).expect("Failed to decode stream id");

            f.debug_struct("OrderedStreamReference")
                .field("messsages_to_send", &format!("... ({})", self.messages_to_send.len()))
                .field("messages_processed", &format!("... ({})", self.messages_processed.len()))
                .field("stream_id", &stream_id)
                .field("lag_cap", &self.lag_cap)
                .field("fin", &self.fin)
                .finish()
        }
    }

    impl std::fmt::Debug for OrderedStreamTransition {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Send(ref stream_message) => match stream_message.message {
                    Some(model::stream_message::Message::Content(_)) => {
                        f.debug_tuple("Send").field(&"Content").finish()
                    }
                    Some(model::stream_message::Message::Fin(_)) => f.debug_tuple("Send").field(&"Fin").finish(),
                    None => panic!("Stream message with None"),
                },
                Self::Receive => f.debug_tuple("Receive").finish(),
            }
        }
    }

    impl ReferenceStateMachine for OrderedStreamReference {
        type State = Self;
        type Transition = OrderedStreamTransition;

        fn init_state() -> BoxedStrategy<Self::State> {
            reference_state_machine().boxed()
        }

        fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
            if let Some(stream_message) = state.messages_to_send.front() {
                prop_oneof![
                    2 => Just(Self::Transition::Send(stream_message.clone())),
                    1 => Just(Self::Transition::Receive),
                ]
                .boxed()
            } else {
                Just(Self::Transition::Receive).boxed()
            }
        }

        fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
            match transition {
                Self::Transition::Send(stream_message) => {
                    assert!(state
                        .messages_processed
                        .insert(stream_message.message_id, stream_message.clone())
                        .is_none());
                    assert!(state.messages_to_send.pop_front().is_some());

                    if let model::stream_message::Message::Fin(_) = stream_message.message.clone().unwrap() {
                        state.fin = Some(stream_message.message_id)
                    };
                }
                Self::Transition::Receive => (),
            }

            state
        }
    }

    impl OrderedStreamReference {
        fn messages_ordered(&self) -> Vec<ConsensusStreamItem> {
            self.messages_processed
                .iter()
                .filter_map(|(id, m)| match m.message.clone().unwrap() {
                    model::stream_message::Message::Content(bytes) => Some(stream_item(&bytes, *id, &self.id_stream)),
                    model::stream_message::Message::Fin(_) => None,
                })
                .collect()
        }
    }

    impl StateMachineTest for SystemUnderTest {
        type SystemUnderTest = Self;
        type Reference = OrderedStreamReference;

        fn init_test(ref_state: &<Self::Reference as ReferenceStateMachine>::State) -> Self::SystemUnderTest {
            let (sx, rx) = ConsensusStreamBuilder::new()
                .with_lag_cap(ref_state.lag_cap)
                .with_channel_cap(1_000)
                .with_id_stream(&ref_state.id_stream)
                .build()
                .expect("Failed to create consensus stream");

            Self { stream_sender: sx, stream_receiver: rx, messages_processed: Default::default() }
        }

        fn apply(
            mut state: Self::SystemUnderTest,
            ref_state: &<Self::Reference as ReferenceStateMachine>::State,
            transition: <Self::Reference as ReferenceStateMachine>::Transition,
        ) -> Self::SystemUnderTest {
            match transition {
                OrderedStreamTransition::Send(stream_message) => {
                    assert_matches::assert_matches!(
                        tokio_test::task::spawn(state.stream_sender.send(stream_message.clone())).poll(),
                        std::task::Poll::Ready(r) => {
                            assert!(r.is_ok());
                        }
                    );
                }
                OrderedStreamTransition::Receive => {
                    let mut fut = tokio_test::task::spawn(state.stream_receiver);
                    while let std::task::Poll::Ready(r) = fut.poll_next() {
                        match r {
                            Some(Ok(proposal)) => state.messages_processed.push(proposal),
                            Some(Err(e)) => panic!("{e}"),
                            None => break,
                        }
                    }

                    let next = fut.poll_next();
                    state.stream_receiver = fut.into_inner();

                    if let std::task::Poll::Ready(None) = next {
                        assert_eq!(state.messages_processed, ref_state.messages_ordered());
                        assert_matches::assert_matches!(state.stream_receiver.state, AccumulatorStateMachine::Done);
                        assert!(state.stream_sender.is_closed());
                    }
                }
            }

            state
        }
    }
}

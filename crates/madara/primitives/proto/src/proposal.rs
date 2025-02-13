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
    #[error("Invalid stream id: expected {1:?}, got {0:?}")]
    InvalidStreamId(StreamId, StreamId),
    #[error("Already received a Fin before this message")]
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
    #[error("The stream has lagged too far behind")]
    LagCap,
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

                let message = inner.next_ordered();
                self.state = AccumulatorStateMachine::Accumulate(inner);

                let Some(message) = message else {
                    let _ = self.waker.lock().expect("Poisoned lock").insert(cx.waker().clone());
                    return Poll::Pending;
                };

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
    // TODO: this needs some more care in regards to DOS migitation:
    //  - limits on the total number of messsages
    fn accumulate_with_force(
        mut self,
        stream_message: model::StreamMessage,
        force: bool,
    ) -> Result<Self, StreamReceiverError<StreamId>> {
        let stream_id = self.stream_id.get_or_insert_with(|| stream_message.stream_id.clone());
        let message_id = stream_message.message_id;
        let last_id = self.messages.last_key_value().map(|(k, _)| *k);

        if !force {
            Self::check_stream_id(&stream_message.stream_id, stream_id)?;
            Self::check_message_id(stream_message.message_id, self.id_low, self.fin)?;
            Self::check_limit(message_id, last_id, self.fin, self.lag_cap)?;
        }

        match model_field!(stream_message => message) {
            stream_message::Message::Content(bytes) => self.update_content(message_id, &bytes),
            stream_message::Message::Fin(..) => self.update_fin(message_id),
        }
    }

    fn accumulate(self, stream_message: model::StreamMessage) -> Result<Self, StreamReceiverError<StreamId>> {
        self.accumulate_with_force(stream_message, false)
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

    fn check_message_id(message_id: u64, id_low: u64, fin: Option<u64>) -> Result<(), StreamReceiverError<StreamId>> {
        if message_id < id_low {
            return Err(StreamReceiverError::DoubleSend(message_id));
        }

        if let Some(fin) = fin {
            if message_id >= fin {
                return Err(StreamReceiverError::SendAfterFin);
            }
        }
        Ok(())
    }

    fn check_limit(
        message_id: u64,
        last_id: Option<u64>,
        fin: Option<u64>,
        lag_cap: u64,
    ) -> Result<(), StreamReceiverError<StreamId>> {
        if let Some(last_id) = last_id {
            let lag = message_id.abs_diff(last_id);
            if lag > lag_cap {
                return Err(StreamReceiverError::LagCap);
            }
        }

        if let Some(fin) = fin {
            let lag = message_id.abs_diff(fin);
            if lag > lag_cap {
                return Err(StreamReceiverError::LagCap);
            }
        }

        Ok(())
    }

    fn update_content(mut self, message_id: u64, bytes: &[u8]) -> Result<Self, StreamReceiverError<StreamId>> {
        match self.messages.entry(message_id) {
            btree_map::Entry::Vacant(entry) => entry.insert(Message::decode(bytes)?),
            btree_map::Entry::Occupied(_) => return Err(StreamReceiverError::DoubleSend(message_id)),
        };

        Ok(self)
    }

    fn update_fin(mut self, fin_id: u64) -> Result<Self, StreamReceiverError<StreamId>> {
        self.fin = match self.fin {
            Some(fin_id_old) => return Err(StreamReceiverError::DoubleFin(fin_id, fin_id_old)),
            None => Some(fin_id),
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
    use futures::{Stream, StreamExt};
    use prost::Message;
    use starknet_core::types::Felt;

    use crate::{
        model::{self},
        proposal::{ConsensusStreamBuilder, StreamReceiverError},
    };

    const STREAM_LEN_DEFAULT: u32 = 10;

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

    #[rstest::fixture]
    fn stream_id(#[default(0)] seed: u32) -> Vec<u8> {
        let stream_id = model::ConsensusStreamId { height: seed as u64, round: seed + 1 };
        model_encode(stream_id)
    }

    #[rstest::fixture]
    fn stream_messages(
        #[default(STREAM_LEN_DEFAULT)] _stream_len: u32,
        #[with(_stream_len)] stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
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
        #[default(STREAM_LEN_DEFAULT)] _stream_len: u32,
        #[with(_stream_len)] stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) -> impl Iterator<Item = model::StreamMessage> {
        stream_messages.collect::<Vec<_>>().into_iter().rev()
    }

    /// Receives a proposal part in a single, ordered stream. All should work as
    /// expected
    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_simple(stream_messages: impl Iterator<Item = model::StreamMessage>) {
        let (mut stream_sender, mut stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

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
        let (mut stream_sender, mut stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

        for message in stream_messages_rev {
            stream_sender.send(message).await.expect("Failed to send stream message");
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

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_pending(
        stream_messages: impl Iterator<Item = model::StreamMessage>,
        stream_messages_rev: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

        let mut fut = tokio_test::task::spawn(stream_receiver);
        for message in stream_messages_rev {
            let message_id = message.message_id;
            stream_sender.send(message).await.expect("Failed to send stream message");

            if message_id != 0 {
                assert_matches::assert_matches!(fut.enter(|cx, rx| rx.poll_next(cx)), std::task::Poll::Pending);
            } else {
                assert!(fut.is_woken());
            }
        }

        for message in stream_messages {
            if let model::stream_message::Message::Content(bytes) = message.message.unwrap() {
                let proposal_part =
                    model::ProposalPart::decode(bytes.as_slice()).expect("Failed to decode proposal part");

                assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(p))) => {
                    assert_eq!(p, proposal_part);
                });
            } else {
                assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None))
            }
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_wake(#[with(2)] mut stream_messages: impl Iterator<Item = model::StreamMessage>) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let message_first = stream_messages.next().unwrap();
        let message_second = stream_messages.next().unwrap();
        let fin = stream_messages.next().unwrap();

        let message = message_first.message.clone().unwrap();
        let model::stream_message::Message::Content(bytes) = message else { unreachable!() };
        let proposal_first = model::ProposalPart::decode(bytes.as_slice()).expect("Failed to decode proposal part");

        let message = message_second.message.clone().unwrap();
        let model::stream_message::Message::Content(bytes) = message else { unreachable!() };
        let proposal_second = model::ProposalPart::decode(bytes.as_slice()).expect("Failed to decode proposal part");

        stream_sender.send(message_second).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(fut.enter(|cx, rx| rx.poll_next(cx)), std::task::Poll::Pending);

        stream_sender.send(message_first).await.expect("Failed to send stream message");
        assert!(fut.is_woken());

        stream_sender.send(fin).await.expect("Failed to send stream message");

        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(p))) => {
            assert_eq!(p, proposal_first);
        });
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(p))) => {
            assert_eq!(p, proposal_second);
        });
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_lag(#[with(5)] mut stream_messages_rev: impl Iterator<Item = model::StreamMessage>) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(3).with_channel_cap(6).build();

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
    async fn ordered_stream_channel_close(#[with(1)] mut stream_messages: impl Iterator<Item = model::StreamMessage>) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let message = stream_messages.next().unwrap();
        let model::stream_message::Message::Content(bytes) = message.message.clone().unwrap() else { unreachable!() };
        let proposal = model::ProposalPart::decode(bytes.as_slice()).expect("Failed to decode proposal part");

        stream_sender.send(message).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(p))) => {
            assert_eq!(p, proposal);
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
    async fn ordered_stream_done(#[with(0)] mut stream_messages: impl Iterator<Item = model::StreamMessage>) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

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
        #[with(2)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let first = stream_messages.next().unwrap();
        let model::stream_message::Message::Content(bytes) = first.message.clone().unwrap() else { unreachable!() };
        let proposal = model::ProposalPart::decode(bytes.as_slice()).expect("Failed to decode proposal part");

        stream_sender.send(first).await.expect("Failed to send proposal part");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(p))) => {
            assert_eq!(p, proposal);
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
        #[with(2)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let first = stream_messages.next().unwrap();
        let model::stream_message::Message::Content(bytes) = first.message.clone().unwrap() else { unreachable!() };
        let proposal = model::ProposalPart::decode(bytes.as_slice()).expect("Failed to decode proposal part");

        stream_sender.send(first).await.expect("Failed to send proposal part");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(p))) => {
            assert_eq!(p, proposal);
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
        #[with(1)] mut stream_messages_rev: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

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
        #[with(1)] mut stream_messages: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

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
        #[from(stream_messages)]
        #[with(0)]
        mut fin: impl Iterator<Item = model::StreamMessage>,
        #[from(stream_messages)]
        #[with(0)]
        mut double_fin: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

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
        #[with(2)] mut stream_messages_rev: impl Iterator<Item = model::StreamMessage>,
    ) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

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
    async fn ordered_stream_double_send_2(#[with(2)] mut stream_messages: impl Iterator<Item = model::StreamMessage>) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

        let mut fut = tokio_test::task::spawn(stream_receiver);

        let first = stream_messages.next().unwrap();
        let model::stream_message::Message::Content(bytes) = first.message.clone().unwrap() else { unreachable!() };
        let proposal = model::ProposalPart::decode(bytes.as_slice()).expect("Failed to decode proposal part");

        stream_sender.send(first).await.expect("Failed to send stream message");
        assert_matches::assert_matches!(fut.poll_next(), std::task::Poll::Ready(Some(Ok(p))) => {
            assert_eq!(p, proposal);
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
    async fn ordered_stream_decode_error(#[with(1)] mut stream_messages: impl Iterator<Item = model::StreamMessage>) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

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
    async fn ordered_stream_model_error(#[with(1)] mut stream_messages: impl Iterator<Item = model::StreamMessage>) {
        let (mut stream_sender, stream_receiver) =
            ConsensusStreamBuilder::new().with_lag_cap(u64::MAX).with_channel_cap(1_000).build();

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

use std::{
    collections::{btree_map, BTreeMap},
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

pub struct OrderedStreamReceiver<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + Default + std::fmt::Debug,
{
    receiver: tokio::sync::mpsc::Receiver<model::StreamMessage>,
    state: AccumulatorStateMachine<Message, StreamId>,
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
    pub fn build(self) -> (tokio::sync::mpsc::Sender<model::StreamMessage>, OrderedStreamReceiver<Message, StreamId>) {
        let Self { lag_cap, channel_cap, stream_id, .. } = self;
        let (sx, rx) = tokio::sync::mpsc::channel(channel_cap);
        let state = AccumulatorStateMachine::new(lag_cap, stream_id);

        let receiver = OrderedStreamReceiver { receiver: rx, state };

        (sx, receiver)
    }
}

impl<Message, StreamId> futures::Stream for OrderedStreamReceiver<Message, StreamId>
where
    Message: prost::Message + std::marker::Unpin + Default,
    StreamId: prost::Message + std::marker::Unpin + Default,
{
    type Item = Result<Message, StreamReceiverError<StreamId>>;

    fn poll_next(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Option<Self::Item>> {
        let Self { receiver, state } = self.get_mut();
        let AccumulatorStateMachine::Accumulate(mut inner) = std::mem::take(state) else {
            return Poll::Ready(None);
        };

        loop {
            if let Some(message) = inner.next_ordered() {
                *state = AccumulatorStateMachine::Accumulate(inner);
                return Poll::Ready(Some(Ok(message)));
            }

            if inner.is_done() {
                receiver.close();
                return Poll::Ready(None);
            }

            let stream_message = match receiver.poll_recv(cx) {
                Poll::Ready(Some(stream_message)) => stream_message,
                Poll::Ready(None) => return Poll::Ready(Some(Err(StreamReceiverError::ChannelError))),
                Poll::Pending => {
                    *state = AccumulatorStateMachine::Accumulate(inner);
                    return Poll::Pending;
                }
            };

            if let Err(e) = inner.accumulate(stream_message) {
                return Poll::Ready(Some(Err(e)));
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
        &mut self,
        stream_message: model::StreamMessage,
        force: bool,
    ) -> Result<(), StreamReceiverError<StreamId>> {
        let stream_id = self.stream_id.get_or_insert_with(|| stream_message.stream_id.clone());
        let message_id = stream_message.message_id;
        let last_id = self.messages.last_key_value().map(|(k, _)| *k);

        if !force {
            Self::check_stream_id(&stream_message.stream_id, stream_id)?;
            Self::check_message_id(stream_message.message_id, self.id_low, self.fin)?;
            Self::check_lag_cap(message_id, last_id, self.fin, self.lag_cap)?;
        }

        match model_field!(stream_message => message) {
            stream_message::Message::Content(bytes) => self.update_content(message_id, &bytes),
            stream_message::Message::Fin(..) => self.update_fin(message_id),
        }
    }

    fn accumulate(&mut self, stream_message: model::StreamMessage) -> Result<(), StreamReceiverError<StreamId>> {
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

    fn check_lag_cap(
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

    fn update_content(&mut self, message_id: u64, bytes: &[u8]) -> Result<(), StreamReceiverError<StreamId>> {
        match self.messages.entry(message_id) {
            btree_map::Entry::Vacant(entry) => entry.insert(Message::decode(bytes)?),
            btree_map::Entry::Occupied(_) => return Err(StreamReceiverError::DoubleSend(message_id)),
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
mod fixtures {

    use starknet_core::types::Felt;

    use crate::model::{self};

    const STREAM_LEN_DEFAULT: u32 = 10;

    #[cfg(test)]
    pub(crate) fn model_encode<M>(proposal_part: M) -> Vec<u8>
    where
        M: prost::Message + Default,
    {
        let mut buffer = Vec::with_capacity(proposal_part.encoded_len());
        proposal_part.encode(&mut buffer).expect("Failed to encode model");

        buffer
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
    pub(crate) fn stream_messages_rev(
        #[default(STREAM_LEN_DEFAULT)] _stream_len: u32,
        #[with(_stream_len)] stream_messages: impl Iterator<Item = model::StreamMessage>,
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
        proposal::{fixtures::*, ConsensusStreamBuilder, StreamReceiverError},
    };

    /// Receives a proposal part in a single, ordered stream. All should work as
    /// expected
    #[tokio::test]
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_millis(100))]
    async fn ordered_stream_simple(stream_messages: impl Iterator<Item = model::StreamMessage>) {
        let (stream_sender, mut stream_receiver) =
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
        let (stream_sender, mut stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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
        let (stream_sender, stream_receiver) =
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

#[cfg(test)]
mod proptest {
    use std::collections::BTreeMap;
    use std::collections::VecDeque;

    use proptest::prelude::*;
    use proptest::prop_compose;
    use proptest_state_machine::ReferenceStateMachine;
    use proptest_state_machine::StateMachineTest;
    use prost::Message;

    use crate::{
        model,
        proposal::{fixtures, OrderedStreamReceiver},
    };

    use super::AccumulatorStateInner;
    use super::AccumulatorStateMachine;
    use super::ConsensusStreamBuilder;

    struct SystemUnderTest {
        stream_sender: tokio::sync::mpsc::Sender<model::StreamMessage>,
        stream_receiver: OrderedStreamReceiver<model::ProposalPart, model::ConsensusStreamId>,
        messages_processed: Vec<model::ProposalPart>,
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

        #[test]
        fn ordered_stream_proptest(sequential 1..160 => SystemUnderTest);
    }

    prop_compose! {
        fn stream_messages()(len in 1..128u32) -> VecDeque<model::StreamMessage> {
            let proposals = fixtures::stream_proposal_part(len);
            let stream_id = fixtures::stream_id(0);
            let messages = fixtures::stream_messages(len, proposals, stream_id);

            return messages.collect()
        }
    }

    prop_compose! {
        fn reference_state_machine()(
            messages_to_send in stream_messages(),
            // lag_cap in 0..256u64
        ) -> OrderedStreamReference {
            OrderedStreamReference {
                messages_to_send,
                messages_processed: Default::default(),
                stream_id: fixtures::stream_id(0),
                lag_cap: 1_000,
                fin: None,
            }
        }
    }

    #[derive(Clone)]
    struct OrderedStreamReference {
        messages_to_send: VecDeque<model::StreamMessage>,
        messages_processed: BTreeMap<u64, model::StreamMessage>,
        stream_id: Vec<u8>,
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
                model::ConsensusStreamId::decode(self.stream_id.as_slice()).expect("Failed to decode stream id");

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
        fn messages_ordered(&self) -> Vec<model::ProposalPart> {
            self.messages_processed
                .values()
                .cloned()
                .filter_map(|m| match m.message.unwrap() {
                    model::stream_message::Message::Content(bytes) => {
                        Some(model::ProposalPart::decode(bytes.as_slice()).expect("Failed to decode proposal part"))
                    }
                    model::stream_message::Message::Fin(_) => None,
                })
                .collect()
        }

        fn check_lag_cap(&self, message_id: u64) -> bool {
            let last_id = self.messages_processed.last_key_value().map(|(k, _)| *k);
            AccumulatorStateInner::<model::ProposalPart, model::ConsensusStreamId>::check_lag_cap(
                message_id,
                last_id,
                self.fin,
                self.lag_cap,
            )
            .is_ok()
        }
    }

    impl StateMachineTest for SystemUnderTest {
        type SystemUnderTest = Self;
        type Reference = OrderedStreamReference;

        fn init_test(ref_state: &<Self::Reference as ReferenceStateMachine>::State) -> Self::SystemUnderTest {
            let (sx, rx) = ConsensusStreamBuilder::new()
                .with_lag_cap(ref_state.lag_cap)
                .with_channel_cap(1_000)
                .with_stream_id(&ref_state.stream_id)
                .build();

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

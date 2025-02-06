use std::collections::BTreeSet;

use m_proc_macros::model_describe;
use prost::Message;

use crate::{
    model::{self, stream_message},
    model_field,
};

#[derive(thiserror::Error, Debug)]
pub enum AccumulateError {
    #[error("Invalid stream id: expected {0:?}, got {1:?}")]
    InvalidStreamId(model::ConsensusStreamId, model::ConsensusStreamId),
    #[error("{0} is more than the max amount of bytes which can be received ({1})")]
    MaxBounds(usize, usize),
    #[error("New Fin with id {1} but already received Fin at message id {0}")]
    DoubleFin(u64, u64),
    #[error("Failed to decode model: {0:?}")]
    DecodeError(#[from] prost::DecodeError),
    #[error(transparent)]
    ModelError(#[from] crate::FromModelError),
}

#[derive(Debug)]
pub enum OrderedStreamAccumulator<T>
where
    T: prost::Message,
    T: Default,
{
    Accumulate(OrderedStreamAccumulatorInner<T>),
    Done(T),
}

#[derive(Debug)]
pub struct OrderedStreamAccumulatorInner<T>
where
    T: prost::Message,
    T: Default,
{
    stream_id: Option<Vec<u8>>,
    messages: BTreeSet<OrderedStreamItem>,
    limits: OrderedStreamLimits,
    fin: Option<u64>,
    _phantom: std::marker::PhantomData<T>,
}

#[derive(Debug)]
struct OrderedStreamItem {
    content: Vec<u8>,
    message_id: u64,
}

impl PartialEq for OrderedStreamItem {
    fn eq(&self, other: &Self) -> bool {
        self.message_id == other.message_id
    }
}

impl Eq for OrderedStreamItem {}

impl Ord for OrderedStreamItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.message_id.cmp(&other.message_id)
    }
}

impl PartialOrd for OrderedStreamItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
struct OrderedStreamLimits {
    max: usize,
    current: usize,
}

impl Default for OrderedStreamLimits {
    fn default() -> Self {
        Self { max: usize::MAX, current: 0 }
    }
}

impl OrderedStreamLimits {
    fn new(max: usize) -> Self {
        Self { max, current: 0 }
    }

    fn update(&mut self, increment: usize) -> Result<(), AccumulateError> {
        if self.current + increment > self.max {
            Err(AccumulateError::MaxBounds(self.current + increment, self.max))
        } else {
            self.current += increment;
            Ok(())
        }
    }
}

impl<T> OrderedStreamAccumulator<T>
where
    T: prost::Message,
    T: Default,
{
    pub fn new() -> Self {
        Self::Accumulate(OrderedStreamAccumulatorInner::<T> {
            stream_id: None,
            messages: Default::default(),
            limits: Default::default(),
            fin: None,
            _phantom: std::marker::PhantomData,
        })
    }

    pub fn new_with_limits(max: usize) -> Self {
        Self::Accumulate(OrderedStreamAccumulatorInner::<T> {
            stream_id: None,
            messages: Default::default(),
            limits: OrderedStreamLimits::new(max),
            fin: None,
            _phantom: std::marker::PhantomData,
        })
    }

    #[model_describe(model::StreamMessage)]
    pub fn accumulate(self, stream_message: model::StreamMessage) -> Result<Self, AccumulateError> {
        match self {
            Self::Accumulate(mut inner) => {
                let stream_id = inner.stream_id.get_or_insert_with(|| stream_message.stream_id.clone());
                let message_id = stream_message.message_id;

                Self::check_stream_id(&stream_message.stream_id, stream_id)?;

                match model_field!(stream_message => message) {
                    stream_message::Message::Content(bytes) => Self::update_content(inner, message_id, &bytes),
                    stream_message::Message::Fin(_) => Self::update_fin(inner, message_id),
                }
            }
            Self::Done(_) => Ok(self),
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done(_))
    }

    pub fn consume(self) -> Option<T> {
        match self {
            Self::Accumulate(_) => None,
            Self::Done(res) => Some(res),
        }
    }

    fn check_stream_id(actual: &[u8], expected: &[u8]) -> Result<(), AccumulateError> {
        if actual != expected {
            let actual = model::ConsensusStreamId::decode(actual)?;
            let expected = model::ConsensusStreamId::decode(expected)?;
            Err(AccumulateError::InvalidStreamId(actual, expected))
        } else {
            Ok(())
        }
    }

    fn update_content(
        mut inner: OrderedStreamAccumulatorInner<T>,
        message_id: u64,
        bytes: &[u8],
    ) -> Result<Self, AccumulateError> {
        inner.limits.update(bytes.len())?;

        let item = OrderedStreamItem { content: bytes.to_vec(), message_id };
        inner.messages.insert(item);

        match inner.fin {
            Some(id) => Self::handle_fin(inner, id),
            None => Ok(Self::Accumulate(inner)),
        }
    }

    fn update_fin(mut inner: OrderedStreamAccumulatorInner<T>, message_id: u64) -> Result<Self, AccumulateError> {
        inner.fin = match inner.fin {
            Some(id) => return Err(AccumulateError::DoubleFin(id, message_id)),
            None => Some(message_id),
        };

        Self::handle_fin(inner, message_id)
    }

    fn handle_fin(inner: OrderedStreamAccumulatorInner<T>, message_id: u64) -> Result<Self, AccumulateError> {
        if inner.messages.len() == message_id as usize {
            let bytes = inner.messages.into_iter().flat_map(|m| m.content).collect::<Vec<_>>();
            Ok(Self::Done(T::decode(bytes.as_slice())?))
        } else {
            Ok(Self::Accumulate(inner))
        }
    }
}

#[cfg(test)]
mod test {
    use prost::Message;
    use rand::{seq::SliceRandom, SeedableRng};
    use starknet_core::types::Felt;

    use crate::{
        model::{self},
        proposal::{AccumulateError, OrderedStreamAccumulator},
    };

    #[rstest::fixture]
    fn proposal_part() -> model::ProposalPart {
        model::ProposalPart {
            messages: Some(model::proposal_part::Messages::Init(model::ProposalInit {
                height: 1,
                round: 2,
                valid_round: Some(3),
                proposer: Some(model::Address(Felt::ONE)),
            })),
        }
    }

    #[rstest::fixture]
    fn stream_proposal_part(
        proposal_part: model::ProposalPart,
    ) -> impl Iterator<Item = model::stream_message::Message> {
        let mut buffer = Vec::new();
        proposal_part.encode(&mut buffer).expect("Failed to encode proposal part");

        buffer
            .chunks(buffer.len() / 10)
            .map(Vec::from)
            .map(model::stream_message::Message::Content)
            .chain(std::iter::once(model::stream_message::Message::Fin(model::Fin {})))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[rstest::fixture]
    fn stream_proposal_part_shuffled(
        stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
    ) -> impl Iterator<Item = model::stream_message::Message> {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(42);
        let mut messages = stream_proposal_part.collect::<Vec<_>>();
        messages.shuffle(&mut rng);
        return messages.into_iter();
    }

    #[rstest::fixture]
    fn stream_id(#[default(0)] seed: u64) -> Vec<u8> {
        let stream_id = model::ConsensusStreamId { height: seed, round: (seed + 1) as u32 };
        let mut stream_id_buffer = Vec::new();
        stream_id.encode(&mut stream_id_buffer).expect("Failed to encode stream id");

        stream_id_buffer
    }

    #[rstest::fixture]
    fn stream_message(
        stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
        #[with(1)] stream_id: Vec<u8>,
    ) -> impl Iterator<Item = model::StreamMessage> {
        stream_proposal_part.enumerate().map(move |(i, message)| model::StreamMessage {
            message: Some(message),
            stream_id: stream_id.clone(),
            message_id: i as u64,
        })
    }

    #[rstest::fixture]
    fn stream_message_shuffled(
        stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
        #[with(1)] stream_id: Vec<u8>,
    ) -> impl Iterator<Item = model::StreamMessage> {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(42);
        let mut stream_messages = stream_proposal_part
            .enumerate()
            .map(move |(i, message)| model::StreamMessage {
                message: Some(message),
                stream_id: stream_id.clone(),
                message_id: i as u64,
            })
            .collect::<Vec<_>>();
        stream_messages.shuffle(&mut rng);

        stream_messages.into_iter()
    }

    #[rstest::fixture]
    fn stream_message_invalid_stream_id(
        mut stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
        #[from(stream_id)]
        #[with(1)]
        stream_id_1: Vec<u8>,
        #[from(stream_id)]
        #[with(2)]
        stream_id_2: Vec<u8>,
    ) -> impl Iterator<Item = model::StreamMessage> {
        vec![
            stream_proposal_part
                .next()
                .map(|message| model::StreamMessage { message: Some(message), stream_id: stream_id_1, message_id: 0 })
                .expect("Failed to generate stream message"),
            stream_proposal_part
                .next()
                .map(|message| model::StreamMessage { message: Some(message), stream_id: stream_id_2, message_id: 0 })
                .expect("Failed to generate stream message"),
        ]
        .into_iter()
    }

    #[rstest::fixture]
    fn stream_message_double_fin(
        stream_message: impl Iterator<Item = model::StreamMessage>,
        #[with(1)] stream_id: Vec<u8>,
    ) -> impl Iterator<Item = model::StreamMessage> {
        std::iter::once(model::StreamMessage {
            message: Some(model::stream_message::Message::Fin(model::Fin {})),
            stream_id: stream_id.clone(),
            message_id: u64::MAX,
        })
        .chain(stream_message)
        .map(move |mut stream_message| {
            stream_message.stream_id = stream_id.clone();
            stream_message
        })
    }

    #[rstest::fixture]
    fn stream_message_decode_error(
        mut stream_proposal_part: impl Iterator<Item = model::stream_message::Message>,
        #[with(1)] stream_id: Vec<u8>,
    ) -> impl Iterator<Item = model::StreamMessage> {
        std::iter::once(model::StreamMessage {
            message: Some(stream_proposal_part.next().unwrap()),
            stream_id: stream_id.clone(),
            message_id: 0,
        })
        .chain(std::iter::once(model::StreamMessage {
            message: Some(model::stream_message::Message::Fin(model::Fin {})),
            stream_id: stream_id.clone(),
            message_id: 0,
        }))
    }

    #[rstest::fixture]
    fn stream_message_model_error(#[with(1)] stream_id: Vec<u8>) -> impl Iterator<Item = model::StreamMessage> {
        std::iter::once(model::StreamMessage { message: None, stream_id, message_id: 0 })
    }

    /// Receives a proposal part in a single, ordered stream. All should work as
    /// expected
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_secs(1))]
    fn ordered_stream_simple(
        proposal_part: model::ProposalPart,
        stream_message: impl Iterator<Item = model::StreamMessage>,
    ) {
        let mut accumulator = OrderedStreamAccumulator::<model::ProposalPart>::new();
        let mut i = 0;

        for message in stream_message {
            accumulator = accumulator.accumulate(message).expect("Failed to accumulate message stream");
            i += 1;
        }

        assert!(i > 1, "Proposal part was streamed over a single message");
        assert!(accumulator.is_done());

        let proposal_part_actual = accumulator.consume();
        assert_eq!(
            proposal_part_actual,
            Some(proposal_part),
            "Failed to reconstruct proposal part from message stream"
        );
    }

    /// Receives a proposal part with a bound to the number of bytes which can
    /// be received.
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_secs(1))]
    fn ordered_stream_bounded(
        proposal_part: model::ProposalPart,
        stream_message: impl Iterator<Item = model::StreamMessage>,
    ) {
        let limit = proposal_part.encode_to_vec().len();
        let mut accumulator = OrderedStreamAccumulator::<model::ProposalPart>::new_with_limits(limit);
        let mut i = 0;

        for message in stream_message {
            accumulator = accumulator.accumulate(message).expect("Failed to accumulate message stream");
            i += 1;
        }

        assert!(i > 1, "Proposal part was streamed over a single message");
        assert!(accumulator.is_done());

        let proposal_part_actual = accumulator.consume();
        assert_eq!(
            proposal_part_actual,
            Some(proposal_part),
            "Failed to reconstruct proposal part from message stream"
        );
    }

    /// Receives a proposal part in an _unordered_ stream. The
    /// [OrderedStreamAccumulator] has to sort the inputs and decode them
    /// correctly.
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_secs(1))]
    fn ordered_stream_shuffled(
        proposal_part: model::ProposalPart,
        stream_message_shuffled: impl Iterator<Item = model::StreamMessage>,
    ) {
        let mut accumulator = OrderedStreamAccumulator::<model::ProposalPart>::new();
        let mut i = 0;

        for message in stream_message_shuffled {
            accumulator = accumulator.accumulate(message).expect("Failed to accumulate message stream");
            i += 1;
        }

        assert!(i > 1, "Proposal part was streamed over a single message");
        assert!(accumulator.is_done());

        let proposal_part_actual = accumulator.consume();
        assert_eq!(
            proposal_part_actual,
            Some(proposal_part),
            "Failed to reconstruct proposal part from message stream"
        );
    }

    /// Receives a proposal part with different stream ids. This is indicative
    /// of multiple streams overlapping and should not happen if the sender is
    /// not malfunctioning.
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_secs(1))]
    fn ordered_stream_fail_invalid_stream_id(
        mut stream_message_invalid_stream_id: impl Iterator<Item = model::StreamMessage>,
    ) {
        let mut accumulator = OrderedStreamAccumulator::<model::ProposalPart>::new();

        accumulator = accumulator
            .accumulate(stream_message_invalid_stream_id.next().unwrap())
            .expect("Failed on first message reception: this should not happen");

        assert_matches::assert_matches!(
            accumulator.accumulate(stream_message_invalid_stream_id.next().unwrap()),
            Err(AccumulateError::InvalidStreamId(..))
        );
    }

    /// Receives a proposal part in a stream with more bytes than is allowed in
    /// the stream limits.
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_secs(1))]
    fn ordered_stream_fail_max_bounds(
        proposal_part: model::ProposalPart,
        stream_message: impl Iterator<Item = model::StreamMessage>,
    ) {
        let limit = proposal_part.encode_to_vec().len();
        let mut accumulator = OrderedStreamAccumulator::<model::ProposalPart>::new_with_limits(limit - 1);

        for message in stream_message {
            accumulator = match accumulator.accumulate(message) {
                Ok(accumulator) => accumulator,
                Err(e) => {
                    assert_matches::assert_matches!(e, AccumulateError::MaxBounds(..));
                    break;
                }
            };
        }
    }

    /// Receives a proposal part in a stream with multiple FIN messages. This is
    /// considered malicious.
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_secs(1))]
    fn ordered_stream_fail_double_fin(stream_message_double_fin: impl Iterator<Item = model::StreamMessage>) {
        let mut accumulator = OrderedStreamAccumulator::<model::ProposalPart>::new();

        for message in stream_message_double_fin {
            accumulator = match accumulator.accumulate(message) {
                Ok(accumulator) => accumulator,
                Err(e) => {
                    assert_matches::assert_matches!(e, AccumulateError::DoubleFin(..));
                    break;
                }
            };
        }
    }

    /// Receives a proposal part in a stream. The proposal part is only
    /// partially sent before the FIN, so this should result in a decode error.
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_secs(1))]
    fn ordered_stream_fail_decode_error(stream_message_decode_error: impl Iterator<Item = model::StreamMessage>) {
        let mut accumulator = OrderedStreamAccumulator::<model::ProposalPart>::new();

        for message in stream_message_decode_error {
            accumulator = match accumulator.accumulate(message) {
                Ok(accumulator) => accumulator,
                Err(e) => {
                    assert_matches::assert_matches!(e, AccumulateError::DecodeError(..));
                    break;
                }
            };
        }
    }

    /// Receives a proposal part in a stream. Protobuf allows for all message
    /// fields to be optional. In our case, we consider any missing field which
    /// is not explicitly marked as `optional` to be required, and return an
    /// error if this is the case.
    #[rstest::rstest]
    #[timeout(std::time::Duration::from_secs(1))]
    fn ordered_stream_fail_model_error(stream_message_model_error: impl Iterator<Item = model::StreamMessage>) {
        let mut accumulator = OrderedStreamAccumulator::<model::ProposalPart>::new();

        for message in stream_message_model_error {
            accumulator = match accumulator.accumulate(message) {
                Ok(accumulator) => accumulator,
                Err(e) => {
                    assert_matches::assert_matches!(e, AccumulateError::ModelError(..));
                    break;
                }
            };
        }
    }
}

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
#[cfg_attr(test, derive(Clone))]
pub enum OrderedStreamAccumulator<T>
where
    T: prost::Message,
    T: Default,
{
    Accumulate(OrderedStreamAccumulatorInner<T>),
    Done(T),
}

#[derive(Debug)]
#[cfg_attr(test, derive(Clone))]
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
#[cfg_attr(test, derive(Clone))]
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
#[cfg_attr(test, derive(Clone))]
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

#[cfg(test)]
mod proptest {
    use std::collections::VecDeque;

    use proptest::prelude::*;
    use proptest::prop_compose;
    use proptest_state_machine::ReferenceStateMachine;
    use proptest_state_machine::StateMachineTest;
    use prost::Message;
    use starknet_core::types::Felt;

    use crate::model;

    use super::AccumulateError;
    use super::OrderedStreamAccumulator;

    type SystemUnderTest = OrderedStreamAccumulator<model::ProposalPart>;

    proptest_state_machine::prop_state_machine! {
        #![proptest_config(proptest::prelude::ProptestConfig {
            // Enable verbose mode to make the state machine test print the
            // transitions for each case.
            verbose: 1,
            // The number of tests which need to be valid for this to pass.
            cases: 64,
            // Max duration (in milliseconds) for each generated case.
            timeout: 1_000,
            ..Default::default()
        })]

        #[test]
        fn ordered_stream_proptest(sequential 1..256 => SystemUnderTest);
    }

    prop_compose! {
        fn stream_id()(seed in 0..100u64) -> Vec<u8> {
            let stream_id = model::ConsensusStreamId { height: seed, round: seed as u32 };
            let mut buffer = Vec::new();
            stream_id.encode(&mut buffer).expect("Failed to encode stream id");

            buffer
        }
    }

    prop_compose! {
        fn proposal_part()(len in 10..100usize) -> model::ProposalPart {
            let tx = model::ConsensusTransaction {
                transaction_hash: Some(model::Hash(Felt::ONE)),
                txn: Some(model::consensus_transaction::Txn::L1Handler(model::L1HandlerV0 {
                    nonce: Some(model::Felt252(Felt::ZERO)),
                    address: Some(model::Address(Felt::ONE)),
                    entry_point_selector: Some(model::Felt252(Felt::TWO)),
                    calldata: vec![model::Felt252(Felt::THREE); 12]
                }))
            };

            model::ProposalPart {
                messages: Some(model::proposal_part::Messages::Transactions(model::TransactionBatch {
                    transactions: vec![tx; len]
                }))
            }
        }
    }

    prop_compose! {
        fn stream_messages(stream_id: Vec<u8>, proposal_part: model::ProposalPart)(
            split_into in 1..256usize
        ) -> VecDeque<model::StreamMessage> {
            let mut buffer = Vec::new();
            proposal_part.encode(&mut buffer).expect("Failed to encode proposal part");

            buffer
                .chunks(buffer.len() / split_into)
                .map(Vec::from)
                .map(model::stream_message::Message::Content)
                .chain(std::iter::once(model::stream_message::Message::Fin(model::Fin {})))
                .enumerate()
                .map(|(i, message)| model::StreamMessage {
                    message: Some(message),
                    stream_id: stream_id.clone(),
                    message_id: i as u64
                })
                .collect()
        }
    }

    prop_compose! {
        fn reference_state_machine()(
            stream_id in stream_id(),
            proposal_part in proposal_part()
        )(
            stream_messages in stream_messages(stream_id.clone(), proposal_part.clone()),
            stream_id in Just(stream_id),
            proposal_part in Just(proposal_part),
            delta in 0..10_000usize
        ) -> OrderedStreamAccumulatorStateMachine {
            let size = proposal_part.encoded_len();
            let limit = if delta > 5_000 {
                size.saturating_sub(delta)
            } else {
                size.saturating_add(delta)
            };

            OrderedStreamAccumulatorStateMachine {
                proposal_part,
                stream_messages,
                stream_id,
                message_id: 0,
                size,
                limit,
            }
        }
    }

    #[derive(Clone)]
    pub enum ProptestTransition {
        Accumulate(model::StreamMessage),
        ActMalicious(ProptestMaliciousTransition),
        Collect,
    }

    #[derive(Clone)]
    pub enum ProptestMaliciousTransition {
        InvalidStreamId(model::StreamMessage),
        InsertGarbageData(model::StreamMessage),
        DoubleFin(model::StreamMessage),
        InvalidModel(model::StreamMessage),
    }

    impl std::fmt::Debug for ProptestTransition {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Accumulate(_) => f.debug_tuple("Accumulate").finish(),
                Self::ActMalicious(transition) => f.debug_tuple("ActMalicious").field(&transition).finish(),
                Self::Collect => f.debug_tuple("Collect").finish(),
            }
        }
    }

    impl std::fmt::Debug for ProptestMaliciousTransition {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::InvalidStreamId(_) => f.debug_tuple("InvalidStreamId").finish(),
                Self::InsertGarbageData(_) => f.debug_tuple("InsertGarbageData").finish(),
                Self::DoubleFin(_) => f.debug_tuple("DoubleFin").finish(),
                Self::InvalidModel(_) => f.debug_tuple("InvalidModel").finish(),
            }
        }
    }

    #[derive(Clone)]
    pub struct OrderedStreamAccumulatorStateMachine {
        proposal_part: model::ProposalPart,
        stream_messages: VecDeque<model::StreamMessage>,
        stream_id: Vec<u8>,
        message_id: u64,
        size: usize,
        limit: usize,
    }

    impl std::fmt::Debug for OrderedStreamAccumulatorStateMachine {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let stream_messages = self
                .stream_messages
                .iter()
                .map(|m| {
                    m.message.clone().map(|m| match m {
                        model::stream_message::Message::Content(_) => "Content(...)",
                        model::stream_message::Message::Fin(_) => "Fin",
                    })
                })
                .take(5)
                .collect::<Vec<_>>();

            let stream_messages = if stream_messages.len() < self.stream_messages.len() {
                format!("{stream_messages:?}... ({} items)", self.stream_messages.len())
            } else {
                format!("{stream_messages:?} ({} items)", self.stream_messages.len())
            };

            f.debug_struct("OrderedStreamAccumulatorStateMachine")
                .field("stream_messages", &stream_messages)
                .field("stream_id", &model::ConsensusStreamId::decode(self.stream_id.as_slice()).unwrap())
                .field("message_id", &self.message_id)
                .field("size", &self.size)
                .field("limit", &self.limit)
                .finish()
        }
    }

    impl ReferenceStateMachine for OrderedStreamAccumulatorStateMachine {
        type State = OrderedStreamAccumulatorStateMachine;
        type Transition = ProptestTransition;

        fn init_state() -> BoxedStrategy<Self::State> {
            reference_state_machine().boxed()
        }

        fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
            if let Some(stream_message) = state.stream_messages.front() {
                if state.message_id == 0 {
                    Just(ProptestTransition::Accumulate(stream_message.clone())).boxed()
                } else {
                    prop_oneof![
                        4 => Just(ProptestTransition::Accumulate(stream_message.clone())),
                        1 => Self::act_malicious(state)
                    ]
                    .boxed()
                }
            } else {
                prop_oneof! [
                    1 => Just(ProptestTransition::Collect),
                    // 1 => Self::act_malicious(state)
                ]
                .boxed()
            }
        }

        fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
            match transition {
                ProptestTransition::Accumulate(_) => {
                    state.stream_messages.pop_front();
                    state.message_id += 1;
                }
                ProptestTransition::ActMalicious(transition) => {
                    if let ProptestMaliciousTransition::InsertGarbageData(_) = transition {
                        state.size += 1;
                    }
                }
                ProptestTransition::Collect => (),
            }

            state
        }
    }

    impl OrderedStreamAccumulatorStateMachine {
        fn act_malicious(state: &Self) -> impl Strategy<Value = ProptestTransition> {
            let invalid_stream_id = || {
                let mut stream_id = model::ConsensusStreamId::decode(state.stream_id.as_slice()).unwrap();
                stream_id.height += 1;
                stream_id.round += 1;

                let mut stream_message = state.stream_messages.front().cloned().unwrap_or_default();
                let mut buffer = Vec::new();
                stream_id.encode(&mut buffer).unwrap();
                stream_message.stream_id = buffer;

                ProptestMaliciousTransition::InvalidStreamId(stream_message)
            };

            let insert_garbage_data = || {
                let content = state
                    .stream_messages
                    .front()
                    .cloned()
                    .unwrap_or_default()
                    .message
                    .unwrap_or(model::stream_message::Message::Content(vec![]));

                let content = if let model::stream_message::Message::Content(mut content) = content {
                    content.push(42);
                    content
                } else {
                    vec![42]
                };

                let stream_message = model::StreamMessage {
                    message: Some(model::stream_message::Message::Content(content)),
                    stream_id: state.stream_id.clone(),
                    message_id: state.message_id,
                };

                ProptestMaliciousTransition::InsertGarbageData(stream_message)
            };

            let double_fin = || {
                let stream_message = model::StreamMessage {
                    message: Some(model::stream_message::Message::Fin(model::Fin {})),
                    stream_id: state.stream_id.clone(),
                    message_id: u64::MAX - state.message_id,
                };

                ProptestMaliciousTransition::DoubleFin(stream_message)
            };

            let invalid_model = || {
                let stream_mesage = model::StreamMessage {
                    message: None,
                    stream_id: state.stream_id.clone(),
                    message_id: u64::MAX / 2 - state.message_id,
                };

                ProptestMaliciousTransition::InvalidModel(stream_mesage)
            };

            prop_oneof![
                Just(ProptestTransition::ActMalicious(invalid_stream_id())),
                // Just(ProptestTransition::ActMalicious(insert_garbage_data())),
                // Just(ProptestTransition::ActMalicious(double_fin())),
                // Just(ProptestTransition::ActMalicious(invalid_model()))
            ]
        }
    }

    impl StateMachineTest for OrderedStreamAccumulator<model::ProposalPart> {
        type SystemUnderTest = Self;
        type Reference = OrderedStreamAccumulatorStateMachine;

        fn init_test(ref_state: &<Self::Reference as ReferenceStateMachine>::State) -> Self::SystemUnderTest {
            Self::new_with_limits(ref_state.limit)
        }

        fn apply(
            mut state: Self::SystemUnderTest,
            ref_state: &<Self::Reference as ReferenceStateMachine>::State,
            transition: <Self::Reference as ReferenceStateMachine>::Transition,
        ) -> Self::SystemUnderTest {
            match transition {
                ProptestTransition::Accumulate(stream_message) => {
                    let res = state.clone().accumulate(stream_message.clone());
                    if !Self::check_limits(&res, stream_message.clone(), &state, ref_state) {
                        assert_matches::assert_matches!(res, Ok(..));
                    }

                    if matches!(stream_message.clone().message.unwrap(), model::stream_message::Message::Fin(_))
                        && ref_state.limit >= ref_state.size
                    {
                        if let Ok(res) = &res {
                            assert_matches::assert_matches!(res, OrderedStreamAccumulator::Done(_));
                        }
                    }

                    // We always set the stream id even in the case of an error.
                    // This insures that subsequent invalid streams ids are seen
                    // as such.
                    if let OrderedStreamAccumulator::Accumulate(mut inner) = state {
                        inner.stream_id = Some(stream_message.stream_id);
                        state = OrderedStreamAccumulator::Accumulate(inner);
                    }

                    res.unwrap_or(state)
                }
                ProptestTransition::ActMalicious(transition) => match transition {
                    ProptestMaliciousTransition::InvalidStreamId(stream_message) => {
                        // We always check for the stream id first! This means
                        // limit checks will not take place on a stream message
                        // with an invalid stream id
                        let res = state.clone().accumulate(stream_message.clone());
                        assert_matches::assert_matches!(res, Err(AccumulateError::InvalidStreamId(..)));
                        res.unwrap_or(state)
                    }
                    ProptestMaliciousTransition::InsertGarbageData(stream_message) => {
                        let res = state.clone().accumulate(stream_message.clone());
                        Self::check_limits(&res, stream_message, &state, ref_state);
                        res.unwrap_or(state)
                    }
                    ProptestMaliciousTransition::DoubleFin(stream_message) => {
                        let res = state.clone().accumulate(stream_message.clone());
                        if !Self::check_limits(&res, stream_message, &state, ref_state) {
                            assert_matches::assert_matches!(res, Err(AccumulateError::DoubleFin(..)));
                        }
                        res.unwrap_or(state)
                    }
                    ProptestMaliciousTransition::InvalidModel(stream_message) => {
                        let res = state.clone().accumulate(stream_message.clone());
                        if !Self::check_limits(&res, stream_message, &state, ref_state) {
                            assert_matches::assert_matches!(res, Err(AccumulateError::ModelError(..)));
                        }
                        res.unwrap_or(state)
                    }
                },
                ProptestTransition::Collect => {
                    if ref_state.limit >= ref_state.size {
                        let res = state.clone().consume();
                        assert!(state.is_done(), "Called collect on incomplete stream");
                        assert!(res.is_some(), "Complete stream returned none");
                        assert_eq!(res, Some(ref_state.proposal_part.clone()), "Collected stream does not match");
                    }
                    state
                }
            }
        }
    }

    impl OrderedStreamAccumulator<model::ProposalPart> {
        fn check_limits(
            res: &Result<Self, AccumulateError>,
            stream_message: model::StreamMessage,
            state: &Self,
            ref_state: &OrderedStreamAccumulatorStateMachine,
        ) -> bool {
            if let OrderedStreamAccumulator::Accumulate(inner) = &state {
                if let model::stream_message::Message::Content(bytes) = stream_message.message.clone().unwrap() {
                    if ref_state.limit < ref_state.size && inner.limits.clone().update(bytes.len()).is_err() {
                        assert_matches::assert_matches!(res, Err(AccumulateError::MaxBounds(..)));
                        return true;
                    }
                }
            }
            false
        }
    }
}

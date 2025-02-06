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
struct OrderedStreamAccumulatorInner<T>
where
    T: prost::Message,
    T: Default,
{
    stream_id: Vec<u8>,
    messages: BTreeSet<OrderedStreamItem>,
    limits: OrderedStreamLimits,
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

#[derive(Default, Debug)]
struct OrderedStreamLimits {
    max: usize,
    current: usize,
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
    pub fn new(stream_id: &[u8]) -> Self {
        Self::Accumulate(OrderedStreamAccumulatorInner::<T> {
            stream_id: stream_id.to_vec(),
            messages: Default::default(),
            limits: Default::default(),
            _phantom: std::marker::PhantomData,
        })
    }

    pub fn new_with_limits(stream_id: &[u8], max: usize) -> Self {
        Self::Accumulate(OrderedStreamAccumulatorInner::<T> {
            stream_id: stream_id.to_vec(),
            messages: Default::default(),
            limits: OrderedStreamLimits::new(max),
            _phantom: std::marker::PhantomData,
        })
    }

    #[model_describe(model::StreamMessage)]
    pub fn accumulate(self, stream_message: model::StreamMessage) -> Result<Self, AccumulateError> {
        match self {
            Self::Accumulate(inner) => {
                Self::check_stream_id(&stream_message.stream_id, &inner.stream_id)?;
                let message_id = stream_message.message_id;

                match model_field!(stream_message => message) {
                    stream_message::Message::Content(bytes) => Self::update_content(inner, &bytes, message_id),
                    stream_message::Message::Fin(_) => Self::update_fin(inner),
                }
            }
            Self::Done(_) => Ok(self),
        }
    }

    pub fn is_done(&self) -> bool {
        return matches!(self, Self::Done(_));
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
        bytes: &[u8],
        message_id: u64,
    ) -> Result<Self, AccumulateError> {
        inner.limits.update(bytes.len())?;

        let item = OrderedStreamItem { content: bytes.to_vec(), message_id };
        inner.messages.insert(item);

        Ok(Self::Accumulate(inner))
    }

    fn update_fin(inner: OrderedStreamAccumulatorInner<T>) -> Result<Self, AccumulateError> {
        let bytes = inner.messages.into_iter().map(|m| m.content).flatten().collect::<Vec<_>>();
        Ok(Self::Done(T::decode(bytes.as_slice())?))
    }
}

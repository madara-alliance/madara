use crate::model;

impl Eq for model::StreamMessage {}

impl Ord for model::StreamMessage {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.message_id.cmp(&other.message_id)
    }
}

impl PartialOrd for model::StreamMessage {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for model::ConsensusSignature {}

// FIXME: Can't really know how to order this until we know the signature schema
impl Ord for model::ConsensusSignature {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        unimplemented!()
    }
}

impl PartialOrd for model::ConsensusSignature {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        unimplemented!()
    }
}

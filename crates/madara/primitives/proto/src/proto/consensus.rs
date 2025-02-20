use crate::model;

impl From<malachite_core_types::VoteType> for model::vote::VoteType {
    fn from(value: malachite_core_types::VoteType) -> Self {
        match value {
            malachite_core_types::VoteType::Prevote => model::vote::VoteType::Prevote,
            malachite_core_types::VoteType::Precommit => model::vote::VoteType::Precommit,
        }
    }
}

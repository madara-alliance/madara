use mp_block::MadaraBlock;

use crate::{
    context::MadaraContext,
    types::{Address, Height},
};
use mp_proto::{model, stream::ConsensusStreamItem};

/// A streamed part of a proposal by a node in the validator set.
///
/// Proposals are never sent in a single piece and instead are streamed over several batches. See
/// [OrderedMessageStream] for more on how this streaming is handled.
///
/// [OrderedMessageStream]: mp_proto::stream::OrderedMessageStream
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct ProposalPart(ConsensusStreamItem);

impl malachite_core_types::ProposalPart<MadaraContext> for ProposalPart {
    fn is_first(&self) -> bool {
        self.0.id_message == 0
    }

    fn is_last(&self) -> bool {
        matches!(self.0.message.messages, Some(model::proposal_part::Messages::Fin(..)))
    }
}

/// A _valid_ proposal in a Tendermint round, made by a node in the validator set.
///
/// # Composing a proposal
///
/// Proposals ares streamed over the network as [ProposalPart]s. A proposer node therefore never
/// sends the entire proposal all at once, but instead batches it by transactions.
///
/// This struct represents the proposal as it is reconstructed by other validator nodes in the
/// network.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Proposal {
    address: Address,
    height: u64,
    round: u32,
    /// From the Tendermint paper:
    ///
    /// > _"lockedRound is the last round in which the process sent a PRECOMMIT message that is not
    /// > nil."_
    round_locked: u32,
    value: MadaraBlock,
}

impl malachite_core_types::Proposal<MadaraContext> for Proposal {
    fn height(&self) -> Height {
        Height(self.height)
    }

    fn round(&self) -> malachite_core_types::Round {
        malachite_core_types::Round::Some(self.round)
    }

    fn value(&self) -> &MadaraBlock {
        &self.value
    }

    fn take_value(self) -> MadaraBlock {
        self.value
    }

    fn pol_round(&self) -> malachite_core_types::Round {
        malachite_core_types::Round::Some(self.round_locked)
    }

    fn validator_address(&self) -> &Address {
        &self.address
    }
}

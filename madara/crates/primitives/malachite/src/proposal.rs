use mp_block::MadaraBlock;

use crate::{
    context::MadaraContext,
    types::{Address, Height},
};
use mp_proto::{model, stream::ConsensusStreamItem};

/// A streamed part of a [Proposal] by a node in the [ValidatorSet].
///
/// [Proposal]s are never sent in a single piece and instead are streamed over several batches. See
/// [OrderedMessageStream] for more on how this streaming is handled.
///
/// [ValidatorSet]: crate::validators::ValidatorSet
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

/// A _valid_ proposal in a Tendermint round, made by a node in the [ValidatorSet].
///
/// # Composing a proposal
///
/// Proposals ares streamed over the network as [ProposalPart]s. A proposer node therefore never
/// sends the entire proposal all at once, but instead batches it by transactions.
///
/// This struct represents the proposal as it is reconstructed by other [Validator] nodes in the
/// network.
///
/// [ValidatorSet]: crate::validators::ValidatorSet
/// [Validator]: crate::validators::Validator
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Proposal {
    address: Address,
    height: Height,
    round: malachite_core_types::Round,
    /// Proof-of-Lock (POL or Polka) is any set of `PREVOTE(r, id(v))` from a super-majority of
    /// [Validator]s, weighted by voting power. This corresponds to `validRound` in the original
    /// Tendermint paper:
    ///
    /// > _"validRound is the last round in which validValue is updated."_
    ///
    /// ie: `valid_round` corresponds to the last round in which a [Validator] received a valid value
    /// `v` along with a super-majority of prevotes (l.36-43). It is possible for a [Validator] to
    /// have received `v` after cating `PREVOTE` [Nil], in which case `v` is stored to be
    /// re-proposed when next the [Validator] becomes a proposer (l.11-21).
    ///
    /// [Validator]: crate::validators::Validator
    /// [Nil]: malachite_core_types::NilOrVal::Nil
    proof_of_lock: malachite_core_types::Round,
    value: MadaraBlock,
}

impl Proposal {
    pub fn new(
        height: Height,
        round: malachite_core_types::Round,
        value: MadaraBlock,
        pol_round: malachite_core_types::Round,
        address: Address,
    ) -> Self {
        Self { address, height, round, proof_of_lock: pol_round, value }
    }
}

impl malachite_core_types::Proposal<MadaraContext> for Proposal {
    fn height(&self) -> Height {
        self.height
    }

    fn round(&self) -> malachite_core_types::Round {
        self.round
    }

    fn value(&self) -> &MadaraBlock {
        &self.value
    }

    fn take_value(self) -> MadaraBlock {
        self.value
    }

    fn pol_round(&self) -> malachite_core_types::Round {
        self.proof_of_lock
    }

    fn validator_address(&self) -> &Address {
        &self.address
    }
}

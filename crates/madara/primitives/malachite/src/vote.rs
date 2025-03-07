//! These are temporary types while I wait to integrate @cchudant's p2p pr into
//! this one.

use crate::{
    context::MadaraContext,
    proposal::{Proposal, ProposalPart},
    types::{Address, Height},
    validators::Validator,
};
use malachite_core_types::Validator as _;
use mp_proto::model;
use starknet_types_core::felt::Felt;

/// A vote cast by a [Validator] in the [ValidatorSet].
///
/// # Vote types
///
/// A [Validator] may either cast a [Val] of [Nil] vote:
///
/// - If the vote is [Val], the [Validator] is committing to the value it received from the proposer
///   for that round (ie: it is marking it as valid and opening itself to slashing if it makes a
///   mistake). This is a succinct representation of the value being voted on, in our case the block
///   hash.
///
/// - If the vote is [Nil], this means the [Validator] disagrees with the value it received from the
///   proposer. This can also happen in the case of a timeout, which is when a round exceeds a max
///   time limit and the required conditions to move on to the next round have not been met.
///
/// # Timeouts
///
/// There are 3 types of timeouts as described in the original Tendermint paper:
///
/// - `TimeoutPropose` occurs when a proposer takes too long to broadcast a block. This results in a
///   node casting a [Precommit] of [Nil], effectively asking to move on to the next [Proposal].
///
/// - `TimeoutPrevote` occurs when a [Validator] has moved on to the [Prevote] round but has not
///   received a super-majority of [Prevote]s in time to start the [Precommit] round. This results
///   in a [Prevote] of [Nil], effectively asking to move on to the [Precommit] round. This is
///   important as it allows [Validator]s which might have lagged behind the network to notice this
///   during the [Precommit] round. See [proof of lock] for more info.
///
/// - `TimeoutPrecommit` occurs when a [Validator] has moved on to the precommit round but does not
///   receive a super-majority of [Precommit]s in time to start the next [Proposal]. This results in a
///   [Precommit] of [Nil], effectively asking to move on to the next [Proposal].
///
/// [Validator]: crate::validators::Validator
/// [ValidatorSet]: crate::validators::ValidatorSet
/// [Val]: malachite_core_types::NilOrVal::Val
/// [Nil]: malachite_core_types::NilOrVal::Nil
/// [Proposal]: crate::proposal::Proposal
/// [Prevote]: malachite_core_types::VoteType::Prevote
/// [Precommit]: malachite_core_types::VoteType::Precommit
/// [proof of lock]: crate::proposal::Proposal::proof_of_lock
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Vote {
    vote_type: malachite_core_types::VoteType,
    vote_value_id: malachite_core_types::NilOrVal<Felt>,
    validator_address: Address,
    height: Height,
    round: malachite_core_types::Round,
}

impl Vote {
    pub fn new_prevote(
        height: Height,
        round: malachite_core_types::Round,
        value_id: malachite_core_types::NilOrVal<Felt>,
        address: Address,
    ) -> Self {
        Self {
            vote_type: malachite_core_types::VoteType::Prevote,
            vote_value_id: value_id,
            validator_address: address,
            height,
            round,
        }
    }

    pub fn new_prevote_nil(height: Height, round: malachite_core_types::Round, address: Address) -> Self {
        Self::new_prevote(height, round, malachite_core_types::NilOrVal::Nil, address)
    }

    pub fn new_precommit(
        height: Height,
        round: malachite_core_types::Round,
        value_id: malachite_core_types::NilOrVal<Felt>,
        address: Address,
    ) -> Self {
        Self {
            vote_type: malachite_core_types::VoteType::Precommit,
            vote_value_id: value_id,
            validator_address: address,
            height,
            round,
        }
    }

    pub fn new_precommit_nil(height: Height, round: malachite_core_types::Round, address: Address) -> Self {
        Self::new_precommit(height, round, malachite_core_types::NilOrVal::Nil, address)
    }

    pub fn is_prevote(&self) -> bool {
        matches!(self.vote_type, malachite_core_types::VoteType::Prevote)
    }

    pub fn is_precommit(&self) -> bool {
        matches!(self.vote_type, malachite_core_types::VoteType::Precommit)
    }

    pub fn vote_type(&self) -> &malachite_core_types::VoteType {
        &self.vote_type
    }
}

impl malachite_core_types::Vote<MadaraContext> for Vote {
    fn height(&self) -> Height {
        self.height
    }

    fn round(&self) -> malachite_core_types::Round {
        self.round
    }

    fn value(&self) -> &malachite_core_types::NilOrVal<Felt> {
        &self.vote_value_id
    }

    fn take_value(self) -> malachite_core_types::NilOrVal<Felt> {
        self.vote_value_id
    }

    fn vote_type(&self) -> malachite_core_types::VoteType {
        self.vote_type
    }

    fn validator_address(&self) -> &Address {
        &self.validator_address
    }

    // INFO: Starknet consensus does not need extensions ATM
    fn extension(&self) -> Option<&malachite_core_types::SignedExtension<MadaraContext>> {
        None
    }

    fn take_extension(&mut self) -> Option<malachite_core_types::SignedExtension<MadaraContext>> {
        None
    }

    // INFO: Starknet consensus does not need extensions ATM
    fn extend(self, _extension: malachite_core_types::SignedExtension<MadaraContext>) -> Self {
        self
    }
}

impl From<Vote> for model::Vote {
    fn from(value: Vote) -> Self {
        Self {
            vote_type: mp_proto::model::vote::VoteType::from(value.vote_type).into(),
            block_number: value.height.block_number,
            fork_id: value.height.fork_id,
            round: value.round.as_u32().unwrap_or_default(),
            block_hash: match value.vote_value_id {
                malachite_core_types::NilOrVal::Val(hash) => Some(model::Hash(hash)),
                malachite_core_types::NilOrVal::Nil => None,
            },
            voter: Some(value.validator_address.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SigningScheme;

impl malachite_core_types::SigningScheme for SigningScheme {
    type DecodingError = String;
    type Signature = model::ConsensusSignature;
    type PublicKey = libp2p::identity::PublicKey;
    type PrivateKey = libp2p::identity::Keypair;

    fn decode_signature(_bytes: &[u8]) -> Result<Self::Signature, Self::DecodingError> {
        // INFO: No signature schema has been defined yet
        // See https://github.com/starknet-io/starknet-p2p-specs/blob/9d7df9b43bdd073a5997b44033bc9dd200362ada/p2p/proto/consensus/protocol.md#signatures
        Ok(Default::default())
    }

    fn encode_signature(_signature: &Self::Signature) -> Vec<u8> {
        // INFO: No signature schema has been defined yet
        Vec::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SigningProvider;

impl malachite_core_types::SigningProvider<MadaraContext> for SigningProvider {
    fn sign_vote(&self, vote: Vote) -> malachite_core_types::SignedMessage<MadaraContext, Vote> {
        // INFO: No signature schema has been defined yet
        malachite_core_types::SignedMessage { message: vote, signature: Default::default() }
    }

    fn verify_signed_vote(
        &self,
        _vote: &Vote,
        _signature: &malachite_core_types::Signature<MadaraContext>,
        _public_key: &malachite_core_types::PublicKey<MadaraContext>,
    ) -> bool {
        // INFO: No signature schema has been defined yet
        true
    }

    fn sign_proposal(&self, proposal: Proposal) -> malachite_core_types::SignedMessage<MadaraContext, Proposal> {
        // INFO: No signature schema has been defined yet
        malachite_core_types::SignedMessage { message: proposal, signature: Default::default() }
    }

    fn verify_signed_proposal(
        &self,
        _proposal: &Proposal,
        _signature: &malachite_core_types::Signature<MadaraContext>,
        _public_key: &malachite_core_types::PublicKey<MadaraContext>,
    ) -> bool {
        // INFO: No signature schema has been defined yet
        true
    }

    fn sign_proposal_part(
        &self,
        proposal_part: ProposalPart,
    ) -> malachite_core_types::SignedMessage<MadaraContext, ProposalPart> {
        // INFO: No signature schema has been defined yet
        malachite_core_types::SignedMessage { message: proposal_part, signature: Default::default() }
    }

    fn verify_signed_proposal_part(
        &self,
        _proposal_part: &ProposalPart,
        _signature: &malachite_core_types::Signature<MadaraContext>,
        _public_key: &malachite_core_types::PublicKey<MadaraContext>,
    ) -> bool {
        // INFO: No signature schema has been defined yet
        true
    }

    fn verify_commit_signature(
        &self,
        _certificate: &malachite_core_types::CommitCertificate<MadaraContext>,
        _commit_sig: &malachite_core_types::CommitSignature<MadaraContext>,
        validator: &Validator,
    ) -> Result<malachite_core_types::VotingPower, malachite_core_types::CertificateError<MadaraContext>> {
        // INFO: No signature schema has been defined yet
        Ok(validator.voting_power())
    }

    fn sign_vote_extension(
        &self,
        _extension: <MadaraContext as malachite_core_types::Context>::Extension,
    ) -> malachite_core_types::SignedMessage<MadaraContext, <MadaraContext as malachite_core_types::Context>::Extension>
    {
        malachite_core_types::SignedMessage { message: (), signature: Default::default() }
    }

    fn verify_signed_vote_extension(
        &self,
        _extension: &<MadaraContext as malachite_core_types::Context>::Extension,
        _signature: &malachite_core_types::Signature<MadaraContext>,
        _public_key: &malachite_core_types::PublicKey<MadaraContext>,
    ) -> bool {
        true
    }
}

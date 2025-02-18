//! These are temporary types while I wait to integrate @cchudant's p2p pr into
//! this one.

use starknet_types_core::felt::Felt;

use crate::{
    context::MadaraContext,
    types::{Address, Height},
};

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
    vote_value: malachite_core_types::NilOrVal<Felt>,
    validator_address: Address,
    height: u64,
    round: u32,
}

impl malachite_core_types::Vote<MadaraContext> for Vote {
    fn height(&self) -> Height {
        Height(self.height)
    }

    fn round(&self) -> malachite_core_types::Round {
        malachite_core_types::Round::Some(self.round)
    }

    fn value(&self) -> &malachite_core_types::NilOrVal<Felt> {
        &self.vote_value
    }

    fn take_value(self) -> malachite_core_types::NilOrVal<Felt> {
        self.vote_value
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

    // INFO: Starknet consensus does not need extensions ATM
    fn extend(self, _extension: malachite_core_types::SignedExtension<MadaraContext>) -> Self {
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SigningSchemeStub;

impl malachite_core_types::SigningScheme for SigningSchemeStub {
    type DecodingError = String;
    type Signature = ();
    type PublicKey = ();
    type PrivateKey = ();

    fn decode_signature(bytes: &[u8]) -> Result<Self::Signature, Self::DecodingError> {
        unimplemented!()
    }

    fn encode_signature(signature: &Self::Signature) -> Vec<u8> {
        unimplemented!()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SigningProviderStub;

impl malachite_core_types::SigningProvider<MadaraContext> for SigningProviderStub {
    fn sign_vote(&self, vote: Vote) -> malachite_core_types::SignedMessage<MadaraContext, Vote> {
        todo!()
    }

    fn verify_signed_vote(
        &self,
        vote: &Vote,
        signature: &malachite_core_types::Signature<MadaraContext>,
        public_key: &malachite_core_types::PublicKey<MadaraContext>,
    ) -> bool {
        todo!()
    }

    fn sign_proposal(
        &self,
        proposal: <MadaraContext as malachite_core_types::Context>::Proposal,
    ) -> malachite_core_types::SignedMessage<MadaraContext, <MadaraContext as malachite_core_types::Context>::Proposal>
    {
        todo!()
    }

    fn verify_signed_proposal(
        &self,
        proposal: &<MadaraContext as malachite_core_types::Context>::Proposal,
        signature: &malachite_core_types::Signature<MadaraContext>,
        public_key: &malachite_core_types::PublicKey<MadaraContext>,
    ) -> bool {
        todo!()
    }

    fn sign_proposal_part(
        &self,
        proposal_part: <MadaraContext as malachite_core_types::Context>::ProposalPart,
    ) -> malachite_core_types::SignedMessage<
        MadaraContext,
        <MadaraContext as malachite_core_types::Context>::ProposalPart,
    > {
        todo!()
    }

    fn verify_signed_proposal_part(
        &self,
        proposal_part: &<MadaraContext as malachite_core_types::Context>::ProposalPart,
        signature: &malachite_core_types::Signature<MadaraContext>,
        public_key: &malachite_core_types::PublicKey<MadaraContext>,
    ) -> bool {
        todo!()
    }

    fn verify_commit_signature(
        &self,
        certificate: &malachite_core_types::CommitCertificate<MadaraContext>,
        commit_sig: &malachite_core_types::CommitSignature<MadaraContext>,
        validator: &<MadaraContext as malachite_core_types::Context>::Validator,
    ) -> Result<malachite_core_types::VotingPower, malachite_core_types::CertificateError<MadaraContext>> {
        todo!()
    }
}

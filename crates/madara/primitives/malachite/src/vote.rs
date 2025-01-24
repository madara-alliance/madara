//! These are temporary types while I wait to integrate @cchudant's p2p pr into
//! this one.

use starknet_types_core::felt::Felt;

use crate::{
    context::MadaraContext,
    types::{Address, Height},
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VoteStub;

impl malachite_core_types::Vote<MadaraContext> for VoteStub {
    fn height(&self) -> Height {
        unimplemented!()
    }

    fn round(&self) -> malachite_core_types::Round {
        unimplemented!()
    }

    fn value(&self) -> &malachite_core_types::NilOrVal<Felt> {
        unimplemented!()
    }

    fn take_value(self) -> malachite_core_types::NilOrVal<Felt> {
        unimplemented!()
    }

    fn vote_type(&self) -> malachite_core_types::VoteType {
        unimplemented!()
    }

    fn validator_address(&self) -> &Address {
        unimplemented!()
    }

    fn extension(&self) -> Option<&malachite_core_types::SignedExtension<MadaraContext>> {
        unimplemented!()
    }

    fn extend(self, extension: malachite_core_types::SignedExtension<MadaraContext>) -> Self {
        unimplemented!()
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
    fn sign_vote(&self, vote: VoteStub) -> malachite_core_types::SignedMessage<MadaraContext, VoteStub> {
        todo!()
    }

    fn verify_signed_vote(
        &self,
        vote: &VoteStub,
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

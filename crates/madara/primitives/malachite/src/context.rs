use mp_block::MadaraBlock;

use crate::{
    proposal::{Proposal, ProposalPart},
    types::{Address, Height},
    validators::{Validator, ValidatorSet},
    vote::{SigningProviderStub, SigningSchemeStub, Vote},
};

#[derive(Clone, Debug)]
pub struct MadaraContext;

impl malachite_core_types::Context for MadaraContext {
    type Address = Address;
    type Height = Height;
    type ProposalPart = ProposalPart;
    type Proposal = Proposal;
    type Validator = Validator;
    type ValidatorSet = ValidatorSet;
    type Value = MadaraBlock;
    type Vote = Vote;
    type SigningScheme = SigningSchemeStub;
    type SigningProvider = SigningProviderStub;

    fn select_proposer<'a>(
        &self,
        validator_set: &'a Self::ValidatorSet,
        height: Self::Height,
        round: malachite_core_types::Round,
    ) -> &'a Self::Validator {
        todo!()
    }

    fn signing_provider(&self) -> &Self::SigningProvider {
        todo!()
    }

    fn new_proposal(
        height: Self::Height,
        round: malachite_core_types::Round,
        value: Self::Value,
        pol_round: malachite_core_types::Round,
        address: Self::Address,
    ) -> Self::Proposal {
        todo!()
    }

    fn new_prevote(
        height: Self::Height,
        round: malachite_core_types::Round,
        value_id: malachite_core_types::NilOrVal<malachite_core_types::ValueId<Self>>,
        address: Self::Address,
    ) -> Self::Vote {
        todo!()
    }

    fn new_precommit(
        height: Self::Height,
        round: malachite_core_types::Round,
        value_id: malachite_core_types::NilOrVal<malachite_core_types::ValueId<Self>>,
        address: Self::Address,
    ) -> Self::Vote {
        todo!()
    }
}

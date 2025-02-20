use mp_block::MadaraBlock;
use starknet_types_core::felt::Felt;

use crate::{
    proposal::{Proposal, ProposalPart},
    types::{Address, Height},
    validators::{Validator, ValidatorSet},
    vote::{SigningProvider, SigningScheme, Vote},
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
    type SigningScheme = SigningScheme;
    type SigningProvider = SigningProvider;

    fn select_proposer<'a>(
        &self,
        validator_set: &'a Self::ValidatorSet,
        height: Self::Height,
        round: malachite_core_types::Round,
    ) -> &'a Self::Validator {
        assert!(validator_set.len() > 0);
        assert!(round != malachite_core_types::Round::Nil);

        let block_number = height.block_number;
        let round = round.as_u32().unwrap_or_default() as u64;
        let len = validator_set.len() as u64;
        let idx = (block_number + round) % len;

        validator_set.validator_get_by_index(idx as usize).expect("Validator at module index exists on a non-empty set")
    }

    fn signing_provider(&self) -> &Self::SigningProvider {
        &SigningProvider
    }

    fn new_proposal(
        height: Self::Height,
        round: malachite_core_types::Round,
        value: Self::Value,
        pol_round: malachite_core_types::Round,
        address: Self::Address,
    ) -> Self::Proposal {
        Proposal::new(height, round, value, pol_round, address)
    }

    fn new_prevote(
        height: Self::Height,
        round: malachite_core_types::Round,
        value_id: malachite_core_types::NilOrVal<Felt>,
        address: Self::Address,
    ) -> Self::Vote {
        Vote::new_prevote(height, round, value_id, address)
    }

    fn new_precommit(
        height: Self::Height,
        round: malachite_core_types::Round,
        value_id: malachite_core_types::NilOrVal<Felt>,
        address: Self::Address,
    ) -> Self::Vote {
        Vote::new_precommit(height, round, value_id, address)
    }
}

use malachite_core_types::Validator as _;
use std::collections::BTreeMap;

use crate::{context::MadaraContext, types::Address};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ValidatorStub;

impl malachite_core_types::Validator<MadaraContext> for ValidatorStub {
    fn address(&self) -> &Address {
        todo!()
    }

    fn public_key(&self) -> &malachite_core_types::PublicKey<MadaraContext> {
        todo!()
    }

    fn voting_power(&self) -> malachite_core_types::VotingPower {
        todo!()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ValidatorSet {
    validators: BTreeMap<Address, ValidatorStub>,
}

impl malachite_core_types::ValidatorSet<MadaraContext> for ValidatorSet {
    fn count(&self) -> usize {
        self.validators.len()
    }

    fn total_voting_power(&self) -> malachite_core_types::VotingPower {
        self.validators.values().fold(0, |acc, v| acc + v.voting_power())
    }

    fn get_by_address(&self, address: &Address) -> Option<&ValidatorStub> {
        self.validators.get(address)
    }

    fn get_by_index(&self, index: usize) -> Option<&ValidatorStub> {
        self.validators.values().take(index).last()
    }
}

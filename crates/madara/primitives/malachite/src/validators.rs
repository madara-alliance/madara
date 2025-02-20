use malachite_core_types::Validator as _;
use std::collections::BTreeMap;

use crate::{context::MadaraContext, types::Address};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Validator {
    address: Address,
    public_key: libp2p::identity::PublicKey,
    voting_power: u64,
}

impl malachite_core_types::Validator<MadaraContext> for Validator {
    fn address(&self) -> &Address {
        &self.address
    }

    fn public_key(&self) -> &libp2p::identity::PublicKey {
        &self.public_key
    }

    fn voting_power(&self) -> malachite_core_types::VotingPower {
        self.voting_power
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ValidatorSet {
    validators: BTreeMap<Address, Validator>,
}

impl ValidatorSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.validators.len()
    }

    pub fn validator_add(&mut self, address: Address, validator: Validator) -> Option<Validator> {
        self.validators.insert(address, validator)
    }

    pub fn validator_remove(&mut self, address: &Address) -> Option<Validator> {
        self.validators.remove(address)
    }

    pub fn validator_get(&self, address: &Address) -> Option<&Validator> {
        self.validators.get(&address)
    }

    pub fn validator_get_by_index(&self, idx: usize) -> Option<&Validator> {
        self.validators.iter().take(idx).last().map(|(_k, v)| v)
    }

    pub fn into_inner(self) -> BTreeMap<Address, Validator> {
        self.validators
    }
}

impl malachite_core_types::ValidatorSet<MadaraContext> for ValidatorSet {
    fn count(&self) -> usize {
        self.validators.len()
    }

    fn total_voting_power(&self) -> malachite_core_types::VotingPower {
        self.validators.values().fold(0, |acc, v| acc + v.voting_power())
    }

    fn get_by_address(&self, address: &Address) -> Option<&Validator> {
        self.validators.get(address)
    }

    fn get_by_index(&self, index: usize) -> Option<&Validator> {
        self.validators.values().take(index).last()
    }
}

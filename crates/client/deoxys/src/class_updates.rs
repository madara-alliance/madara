use hex::decode;
use mp_felt::Felt252Wrapper;
use parity_scale_codec::{Encode, Decode};
use starknet_providers::sequencer::models::{StateUpdate, state_update::DeclaredContract};

/// A structure which holds data about contract class updates from the sequencer.
/// This is used to query the sequencer for new contract classes if they are not
/// already stored in local db.
#[derive(Debug, Default)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
pub struct StarknetClassUpdate {
    pub contract_updates: Vec<Felt252Wrapper>,
}

impl From<&StateUpdate> for StarknetClassUpdate {
    fn from(state_update: &StateUpdate) -> Self {
        let mut class_update = StarknetClassUpdate::default();

        for (address, _) in &state_update
            .state_diff
            .storage_diffs
        {
            class_update.contract_updates.push(Felt252Wrapper::from(address.clone()));
        }

        // retrieves new Cairo v1 classes
        for DeclaredContract { class_hash, compiled_class_hash: _ } in &state_update
            .state_diff
            .declared_classes
        {
            class_update.contract_updates.push(Felt252Wrapper::from(class_hash.clone()));
        }

        // retrieves new Cairo v0 classes
        for class_hash in &state_update
            .state_diff
            .old_declared_contracts
        {
            class_update.contract_updates.push(Felt252Wrapper::from(class_hash.clone()));
        }

        class_update
    }
}

impl Encode for StarknetClassUpdate {
    fn size_hint(&self) -> usize {
        self.contract_updates.iter().map(|f| f.size_hint()).sum()
    }

    fn encode_to<T: parity_scale_codec::Output + ?Sized>(&self, dest: &mut T) {
        self.contract_updates.encode_to(dest);
    }
}

impl Decode for StarknetClassUpdate {
    fn decode<I: parity_scale_codec::Input>(input: &mut I) -> Result<Self, parity_scale_codec::Error> {
        Ok(StarknetClassUpdate {
            contract_updates: Vec::<Felt252Wrapper>::decode(input)?
        })
    }
}
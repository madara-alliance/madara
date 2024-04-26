use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_ff::FieldElement;

pub trait FromFieldElement<T> {
    fn from_field_element(value: T) -> Self;
}

impl FromFieldElement<FieldElement> for ClassHash {
    fn from_field_element(value: FieldElement) -> Self {
        Self(StarkFelt::from_field_element(value))
    }
}

impl FromFieldElement<&FieldElement> for ClassHash {
    fn from_field_element(value: &FieldElement) -> Self {
        Self(StarkFelt::from_field_element(value))
    }
}

impl FromFieldElement<FieldElement> for CompiledClassHash {
    fn from_field_element(value: FieldElement) -> Self {
        Self(StarkFelt::from_field_element(value))
    }
}

impl FromFieldElement<&FieldElement> for CompiledClassHash {
    fn from_field_element(value: &FieldElement) -> Self {
        Self(StarkFelt::from_field_element(value))
    }
}

impl FromFieldElement<FieldElement> for ContractAddress {
    fn from_field_element(value: FieldElement) -> Self {
        Self(PatriciaKey::from_field_element(value))
    }
}

impl FromFieldElement<&FieldElement> for ContractAddress {
    fn from_field_element(value: &FieldElement) -> Self {
        Self(PatriciaKey::from_field_element(value))
    }
}

impl FromFieldElement<FieldElement> for Nonce {
    fn from_field_element(value: FieldElement) -> Self {
        Nonce(StarkFelt::from_field_element(value))
    }
}

impl FromFieldElement<&FieldElement> for Nonce {
    fn from_field_element(value: &FieldElement) -> Self {
        Nonce(StarkFelt::from_field_element(value))
    }
}

impl FromFieldElement<FieldElement> for PatriciaKey {
    fn from_field_element(value: FieldElement) -> Self {
        Self(StarkFelt::from_field_element(value))
    }
}

impl FromFieldElement<&FieldElement> for PatriciaKey {
    fn from_field_element(value: &FieldElement) -> Self {
        Self(StarkFelt::from_field_element(value))
    }
}

impl FromFieldElement<FieldElement> for StarkFelt {
    fn from_field_element(value: FieldElement) -> Self {
        Self::new_unchecked(value.to_bytes_be())
    }
}

impl FromFieldElement<&FieldElement> for StarkFelt {
    fn from_field_element(value: &FieldElement) -> Self {
        Self::new_unchecked(value.to_bytes_be())
    }
}

impl FromFieldElement<FieldElement> for StorageKey {
    fn from_field_element(value: FieldElement) -> Self {
        Self(PatriciaKey::from_field_element(value))
    }
}

impl FromFieldElement<&FieldElement> for StorageKey {
    fn from_field_element(value: &FieldElement) -> Self {
        Self(PatriciaKey::from_field_element(value))
    }
}

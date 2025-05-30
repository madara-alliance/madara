use crate::types::batch::{ClassDeclaration, ContractUpdate, DataJson, StorageUpdate};
use color_eyre::Result;
use num_bigint::BigUint;
use num_traits::Zero;
use std::collections::HashSet;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ComparisonError {
    #[error("state_update_size differs: {0} vs {1}")]
    StateUpdateSizeMismatch(u64, u64),

    #[error("class_declaration_size differs: {0} vs {1}")]
    ClassDeclarationSizeMismatch(u64, u64),

    #[error("state_update count differs: {0} vs {1}")]
    StateUpdateCountMismatch(usize, usize),

    #[error("class_declaration count differs: {0} vs {1}")]
    ClassDeclarationCountMismatch(usize, usize),

    #[error("missing contract update with address {0}")]
    MissingContractUpdate(BigUint),

    #[error("contract update with address {0} differs: nonce {1} vs {2}")]
    ContractUpdateNonceMismatch(BigUint, u64, u64),

    #[error("contract update with address {0} differs: storage_updates count {1} vs {2}")]
    ContractUpdateStorageCountMismatch(BigUint, u64, u64),

    #[error("contract update with address {0} differs: new_class_hash {1:?} vs {2:?}")]
    ContractUpdateClassHashMismatch(BigUint, Option<BigUint>, Option<BigUint>),

    #[error("contract update with address {0} has different storage updates")]
    ContractUpdateStorageMismatch(BigUint),

    #[error("missing class declaration with hash {0}")]
    MissingClassDeclaration(BigUint),
}

pub fn compare_data_json(a: &DataJson, b: &DataJson) -> Result<(), ComparisonError> {
    // Compare sizes
    if a.state_update_size != b.state_update_size {
        return Err(ComparisonError::StateUpdateSizeMismatch(a.state_update_size, b.state_update_size));
    }

    if a.class_declaration_size != b.class_declaration_size {
        return Err(ComparisonError::ClassDeclarationSizeMismatch(a.class_declaration_size, b.class_declaration_size));
    }

    // Compare state_update vectors (order doesn't matter)
    if a.state_update.len() != b.state_update.len() {
        return Err(ComparisonError::StateUpdateCountMismatch(a.state_update.len(), b.state_update.len()));
    }

    // Create a map from address to ContractUpdate for both a and b
    let a_updates: std::collections::HashMap<&BigUint, &ContractUpdate> =
        a.state_update.iter().map(|cu| (&cu.address, cu)).collect();

    let b_updates: std::collections::HashMap<&BigUint, &ContractUpdate> =
        b.state_update.iter().map(|cu| (&cu.address, cu)).collect();

    // Check that all addresses in a are also in b
    for (address, a_update) in &a_updates {
        // Check if this address exists in b
        let b_update = match b_updates.get(address) {
            Some(update) => update,
            None => return Err(ComparisonError::MissingContractUpdate((*address).clone())),
        };

        // Compare the ContractUpdate fields
        if a_update.nonce != b_update.nonce {
            return Err(ComparisonError::ContractUpdateNonceMismatch(
                (*address).clone(),
                a_update.nonce,
                b_update.nonce,
            ));
        }

        if a_update.number_of_storage_updates != b_update.number_of_storage_updates {
            return Err(ComparisonError::ContractUpdateStorageCountMismatch(
                (*address).clone(),
                a_update.number_of_storage_updates,
                b_update.number_of_storage_updates,
            ));
        }

        if a_update.new_class_hash != b_update.new_class_hash {
            return Err(ComparisonError::ContractUpdateClassHashMismatch(
                (*address).clone(),
                a_update.new_class_hash.clone(),
                b_update.new_class_hash.clone(),
            ));
        }

        // Compare storage updates (order doesn't matter)
        let a_storage_set: HashSet<&StorageUpdate> = a_update.storage_updates.iter().collect();
        let b_storage_set: HashSet<&StorageUpdate> = b_update.storage_updates.iter().collect();

        if a_storage_set != b_storage_set {
            return Err(ComparisonError::ContractUpdateStorageMismatch((*address).clone()));
        }
    }

    // Check that all addresses in b are also in a
    // (This is technically redundant because we already checked that they have the same length
    // and that all addresses in a are in b, but including for clarity)
    for address in b_updates.keys() {
        if !a_updates.contains_key(address) {
            return Err(ComparisonError::MissingContractUpdate((*address).clone()));
        }
    }

    // Compare class_declaration vectors (order doesn't matter)
    if a.class_declaration.len() != b.class_declaration.len() {
        return Err(ComparisonError::ClassDeclarationCountMismatch(
            a.class_declaration.len(),
            b.class_declaration.len(),
        ));
    }

    // Since ClassDeclaration derives Hash and Eq, we can use HashSet to compare
    let a_class_set: HashSet<&ClassDeclaration> = a.class_declaration.iter().collect();
    let b_class_set: HashSet<&ClassDeclaration> = b.class_declaration.iter().collect();

    // Find missing class declarations
    for class_decl in a_class_set.difference(&b_class_set) {
        return Err(ComparisonError::MissingClassDeclaration(class_decl.class_hash.clone()));
    }

    for class_decl in b_class_set.difference(&a_class_set) {
        return Err(ComparisonError::MissingClassDeclaration(class_decl.class_hash.clone()));
    }

    // If we reach here, the two DataJson objects are considered equal
    Ok(())
}

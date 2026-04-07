use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};

/// A trait for simple hash functions used in proof verification.
///
/// This trait provides a unified interface for hash functions that take two `Felt` values
/// and return a single `Felt` hash. It's used primarily in the proof verification system.
///
/// # Example
///
/// ```rust
/// use rpc_client::types::Hash;
/// use starknet_types_core::felt::Felt;
///
/// struct MyHashFunction;
///
/// impl SimpleHashFunction for MyHashFunction {
///     fn hash(left: &Felt, right: &Felt) -> Felt {
///         // Implement your hash function here
///         Felt::from(0)
///     }
/// }
/// ```
pub trait Hash {
    /// Computes a hash from two input values.
    ///
    /// # Arguments
    ///
    /// * `left` - The left input value
    /// * `right` - The right input value
    ///
    /// # Returns
    ///
    /// The computed hash value.
    fn hash(left: &Felt, right: &Felt) -> Felt;
}

pub struct PedersenHash;
impl Hash for PedersenHash {
    fn hash(left: &Felt, right: &Felt) -> Felt {
        Pedersen::hash(left, right)
    }
}

/// Implementation for Poseidon hash
pub struct PoseidonHash;
impl Hash for PoseidonHash {
    fn hash(left: &Felt, right: &Felt) -> Felt {
        Poseidon::hash(left, right)
    }
}

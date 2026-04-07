pub mod class;
pub mod contract;

#[cfg(test)]
mod tests;

pub use class::ClassProof;
pub use contract::{ContractData, ContractProof, Height};

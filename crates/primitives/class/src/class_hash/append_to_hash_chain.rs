use dp_hash_chain::HashChain;
use starknet_core::types::{Felt, LegacyContractEntryPoint, LegacyEntryPointsByType};
use starknet_types_core::hash::StarkHash;

use super::cairo_program::{Builtins, Data};

/// A way to include complex structs into an existing HashChain
pub trait AppendToHashChain<SH: StarkHash> {
    type Error;

    fn append_to_hash_chain(&self, outer: &mut HashChain<SH>) -> Result<(), Self::Error>;
}

pub struct LegacyEntryPointsByTypeHashChainWrapper<'a>(pub &'a LegacyEntryPointsByType);

impl<'a, SH: StarkHash> AppendToHashChain<SH> for LegacyEntryPointsByTypeHashChainWrapper<'a> {
    type Error = anyhow::Error;

    fn append_to_hash_chain(&self, outer: &mut HashChain<SH>) -> Result<(), Self::Error> {
        let mut hasher = HashChain::<SH>::default();
        for LegacyContractEntryPoint { selector, offset } in &self.0.external {
            hasher.update(selector);
            hasher.update(&Felt::from(*offset));
        }
        outer.update(&hasher.finalize());

        let mut hasher = HashChain::<SH>::default();
        for LegacyContractEntryPoint { selector, offset } in &self.0.l1_handler {
            hasher.update(selector);
            hasher.update(&Felt::from(*offset));
        }
        outer.update(&hasher.finalize());

        let mut hasher = HashChain::<SH>::default();
        for LegacyContractEntryPoint { selector, offset } in &self.0.constructor {
            hasher.update(selector);
            hasher.update(&Felt::from(*offset));
        }
        outer.update(&hasher.finalize());

        Ok(())
    }
}

impl<'a, SH: StarkHash> AppendToHashChain<SH> for Builtins<'a> {
    type Error = anyhow::Error;

    fn append_to_hash_chain(&self, outer: &mut HashChain<SH>) -> Result<(), Self::Error> {
        let mut hasher = HashChain::<SH>::default();
        for v in self.0.iter().map(|s| Felt::from_bytes_be_slice(s.as_bytes())) {
            hasher.update(&v);
        }

        outer.update(&hasher.finalize());

        Ok(())
    }
}

impl<'a, SH: StarkHash> AppendToHashChain<SH> for Data<'a> {
    type Error = anyhow::Error;

    fn append_to_hash_chain(&self, outer: &mut HashChain<SH>) -> Result<(), Self::Error> {
        let mut hasher = HashChain::<SH>::default();
        for v in self.0.iter().map(|s| Felt::from_hex(s).unwrap()) {
            hasher.update(&v);
        }

        outer.update(&hasher.finalize());

        Ok(())
    }
}

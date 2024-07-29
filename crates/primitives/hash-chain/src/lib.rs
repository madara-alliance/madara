use std::marker::PhantomData;

use starknet_types_core::felt::Felt;
use starknet_types_core::hash::StarkHash;

pub struct HashChain<SH>
where
    SH: StarkHash,
{
    hash: Felt,
    count: usize,
    stark_hash: PhantomData<SH>,
}

impl<SH: StarkHash> Default for HashChain<SH> {
    fn default() -> Self {
        Self { hash: Default::default(), count: Default::default(), stark_hash: PhantomData }
    }
}

impl<SH: StarkHash> HashChain<SH> {
    pub fn update(&mut self, value: &Felt) {
        self.hash = SH::hash(&self.hash, value);
        self.count += 1;
    }

    pub fn finalize(self) -> Felt {
        SH::hash(&self.hash, &Felt::from(self.count))
    }
    pub fn state(&self) -> (String, usize) {
        (self.hash.to_hex_string(), self.count)
    }
}

#[cfg(test)]
mod tests {
    use super::{Felt, HashChain};
    use starknet_types_core::hash::Pedersen;

    #[test]
    fn test_non_empty_chain() {
        let mut chain = HashChain::<Pedersen>::default();

        chain.update(&Felt::from_hex("0x1").unwrap());
        chain.update(&Felt::from_hex("0x2").unwrap());
        chain.update(&Felt::from_hex("0x3").unwrap());
        chain.update(&Felt::from_hex("0x4").unwrap());

        let computed_hash = chain.finalize();

        // produced by the cairo-lang Python implementation:
        // `hex(compute_hash_on_elements([1, 2, 3, 4]))`
        let expected_hash =
            Felt::from_hex("0x66bd4335902683054d08a0572747ea78ebd9e531536fb43125424ca9f902084").unwrap();

        assert_eq!(expected_hash, computed_hash);
    }
}

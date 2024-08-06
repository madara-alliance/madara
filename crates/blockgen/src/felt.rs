use rand::{rngs::StdRng, Rng};
use starknet_types_core::felt::Felt;

pub enum FeltMax {
    U64,
    U128,
    // PatriciaKey,
    Max,
}

pub(crate) fn random_felt(rng: &mut StdRng, max: FeltMax) -> Felt {
    match max {
        FeltMax::U64 => rng.gen::<u64>().into(),
        FeltMax::U128 => rng.gen::<u128>().into(),
        // FeltMax::PatriciaKey => {
        //     let mut bytes: [u8; 32] = rng.gen();
        //     bytes[0] &= 0b0000_1111;
        //     Felt::from_bytes_be_slice(&bytes)
        // }
        FeltMax::Max => {
            let mut bytes: [u8; 32] = rng.gen();
            bytes[0] |= 0b0000_1111;
            Felt::from_bytes_be_slice(&bytes)
        }
    }
}

pub(crate) fn random_felts(rng: &mut StdRng, n: usize) -> Vec<Felt> {
    (0..n).map(|_| random_felt(rng, FeltMax::Max)).collect()
}

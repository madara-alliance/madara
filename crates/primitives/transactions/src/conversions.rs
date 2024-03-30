use alloc::sync::Arc;
use alloc::vec::Vec;

use mp_felt::Felt252Wrapper;
use starknet_api::transaction as sttx;

fn vec_of_felt_to_signature(felts: &[Felt252Wrapper]) -> sttx::TransactionSignature {
    sttx::TransactionSignature(felts.iter().map(|&f| f.into()).collect())
}

fn vec_of_felt_to_calldata(felts: &[Felt252Wrapper]) -> sttx::Calldata {
    sttx::Calldata(Arc::new(felts.iter().map(|&f| f.into()).collect()))
}

// Utility function to convert `TransactionSignature` into a vector of `Felt252Wrapper`
fn signature_to_vec_of_felt(sig: &sttx::TransactionSignature) -> Vec<Felt252Wrapper> {
    sig.0.iter().map(|&f| Felt252Wrapper::from(f)).collect()
}

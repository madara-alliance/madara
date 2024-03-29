use alloc::sync::Arc;
use alloc::vec::Vec;

use mp_felt::Felt252Wrapper;
use starknet_api::transaction as sttx;

use super::HandleL1MessageTransaction;

impl HandleL1MessageTransaction {
    // pub fn into_executable<H: HasherT>(
    //     &self,
    //     chain_id: Felt252Wrapper,
    //     paid_fee_on_l1: Fee,
    //     offset_version: bool,
    // ) -> btx::L1HandlerTransaction { let transaction_hash = self.compute_hash::<H>(chain_id,
    //   offset_version, None);

    //     let tx = sttx::L1HandlerTransaction {
    //         version: TransactionVersion(StarkFelt::from(0u8)),
    //         nonce: Nonce(StarkFelt::from(self.nonce)),
    //         contract_address: self.contract_address.into(),
    //         entry_point_selector: self.entry_point_selector.into(),
    //         calldata: vec_of_felt_to_calldata(&self.calldata),
    //     };

    //     btx::L1HandlerTransaction { tx, paid_fee_on_l1, tx_hash: transaction_hash.into() }
    // }

    pub fn from_starknet(inner: starknet_api::transaction::L1HandlerTransaction) -> Self {
        Self {
            nonce: u64::try_from(inner.nonce.0).unwrap(),
            contract_address: inner.contract_address.into(),
            entry_point_selector: inner.entry_point_selector.0.into(),
            calldata: inner.calldata.0.iter().map(|felt| Felt252Wrapper::from(*felt)).collect(),
        }
    }
}

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

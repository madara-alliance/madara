#![cfg(test)]

use crate::{MadaraBackend, RejectedMempoolTransaction};
use mp_chain_config::ChainConfig;
use mp_convert::Felt;

#[test]
fn rejected_mempool_transaction_roundtrip() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let hash = Felt::from(123u64);
    let rejection = RejectedMempoolTransaction::new("too large for proof");

    backend.write_rejected_mempool_transaction(&hash, &rejection).unwrap();

    assert_eq!(backend.get_rejected_mempool_transaction(&hash).unwrap(), Some(rejection));
}

#[test]
fn l1_message_proof_quarantine_roundtrip_and_clear() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    backend.write_l1_message_proof_rejection(55, "too large for proof").unwrap();
    assert_eq!(backend.get_l1_message_proof_rejection(55).unwrap().as_deref(), Some("too large for proof"));

    backend.clear_l1_message_proof_rejection(55).unwrap();
    assert_eq!(backend.get_l1_message_proof_rejection(55).unwrap(), None);
}

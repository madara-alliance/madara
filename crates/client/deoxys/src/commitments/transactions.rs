use std::sync::Arc;

use bitvec::prelude::*;
use bonsai_trie::bonsai_database::DatabaseKey;
use bonsai_trie::id::{BasicId, BasicIdBuilder};
use bonsai_trie::{BonsaiDatabase, BonsaiStorage, BonsaiStorageConfig, BonsaiTrieHash, Membership, ProofNode};
use mc_db::bonsai_db::BonsaiDb;
use mc_db::{Backend, BonsaiDbError};
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::{DeployTransaction, InvokeTransactionV0, Transaction};
use parity_scale_codec::{Decode, Encode};
use sp_core::H256;
use sp_runtime::traits::Block as BlockT;
use starknet_api::hash::StarkFelt;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;

/// Compute the combined hash of the transaction hash and the signature.
///
/// Since the transaction hash doesn't take the signature values as its input
/// computing the transaction commitent uses a hash value that combines
/// the transaction hash with the array of signature values.
///
/// # Arguments
///
/// * `tx` - The transaction to compute the hash of.
///
/// # Returns
///
/// The transaction hash with signature.
pub fn calculate_transaction_hash_with_signature<H: HasherT>(
    tx: &Transaction,
    chain_id: Felt252Wrapper,
    block_number: u64,
) -> FieldElement
where
    H: HasherT,
{
    let include_signature = block_number >= 61394;

    let signature_hash = if matches!(tx, Transaction::Invoke(_)) || include_signature {
        // Include signatures for Invoke transactions or for all transactions
        // starting from block 61394
        H::compute_hash_on_elements(
            &tx.signature().iter().map(|elt| FieldElement::from(*elt)).collect::<Vec<FieldElement>>(),
        )
    } else {
        // Before block 61394, and for non-Invoke transactions, signatures are not included
        H::compute_hash_on_elements(&[])
    };

    let transaction_hashes =
        H::hash_elements(FieldElement::from(tx.compute_hash::<H>(chain_id, false, Some(block_number))), signature_hash);

    transaction_hashes
}

pub(crate) fn calculate_transaction_commitment<B, H>(
    transactions: &[Transaction],
    chain_id: Felt252Wrapper,
    block_number: u64,
    backend: &Arc<BonsaiDb<B>>,
) -> Result<Felt252Wrapper, BonsaiDbError>
where
    B: BlockT,
    H: HasherT,
{
    let config = BonsaiStorageConfig::default();
    let bonsai_db = backend.as_ref();
    let mut bonsai_storage =
        BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    let transactions: Vec<Transaction> = vec![
        mp_transactions::Transaction::Deploy(DeployTransaction {
            version: starknet_api::transaction::TransactionVersion(StarkFelt(
                Felt252Wrapper::from_hex_be("0x0000000000000000000000000000000000000000000000000000000000000000")
                    .unwrap()
                    .into(),
            )),
            class_hash: Felt252Wrapper::from_hex_be(
                "0x010455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8",
            )
            .unwrap(),
            contract_address_salt: Felt252Wrapper::from_hex_be(
                "0x03a6b18fc3415b7d749f18483393b0d6a1aef168435016c0f5f5d8902a84a36f",
            )
            .unwrap(),
            constructor_calldata: vec![
                Felt252Wrapper::from_hex_be("0x04184fa5a6d40f47a127b046ed6facfa3e6bc3437b393da65cc74afe47ca6c6e")
                    .unwrap(),
                Felt252Wrapper::from_hex_be("0x001ef78e458502cd457745885204a4ae89f3880ec24db2d8ca97979dce15fedc")
                    .unwrap(),
            ],
        }),
        mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V0(InvokeTransactionV0 {
            max_fee: 0,
            signature: vec![],
            contract_address: mp_felt::Felt252Wrapper::from_hex_be(
                "0x06538fdd3aa353af8a87f5fe77d1f533ea82815076e30a86d65b72d3eb4f0b80",
            )
            .unwrap(),
            entry_point_selector: mp_felt::Felt252Wrapper::from_hex_be(
                "0x0218f305395474a84a39307fa5297be118fe17bf65e27ac5e2de6617baa44c64",
            )
            .unwrap(),
            calldata: vec![
                mp_felt::Felt252Wrapper::from_hex_be(
                    "0x0327d34747122d7a40f4670265b098757270a449ec80c4871450fffdab7c2fa8",
                )
                .unwrap(),
                mp_felt::Felt252Wrapper::ZERO,
            ],
        })),
    ];

    for (i, tx) in transactions.iter().enumerate() {
        let tx_hash = calculate_transaction_hash_with_signature::<H>(tx, chain_id, block_number);
        let key = BitVec::from_vec(i.to_be_bytes().to_vec());
        let felt_value = Felt::from(Felt252Wrapper::from(tx_hash));
        bonsai_storage.insert(key.as_bitslice(), &felt_value).expect("Failed to insert into bonsai storage");
    }

    let mut id_builder = BasicIdBuilder::new();
    let id = id_builder.new_id();
    bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");

    let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
    println!("Transaction commitment: {:?}", Felt252Wrapper::from(root_hash));

    Ok(Felt252Wrapper::from(root_hash))
}

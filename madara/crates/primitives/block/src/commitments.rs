use crate::{header::PreconfirmedHeader, Header, TransactionWithReceipt};
use bitvec::vec::BitVec;
use mp_chain_config::StarknetVersion;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::StateDiff;
use rayon::iter::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};
use starknet_types_core::{
    felt::Felt,
    hash::{Pedersen, Poseidon, StarkHash},
};

pub struct CommitmentComputationContext {
    pub protocol_version: StarknetVersion,
    pub chain_id: Felt,
}

pub struct TransactionAndReceiptCommitment {
    pub transaction_commitment: Felt,
    pub receipt_commitment: Felt,
    pub transaction_count: u64,
}

impl TransactionAndReceiptCommitment {
    /// Uses the rayon pool.
    pub fn compute(ctx: &CommitmentComputationContext, transactions: &[TransactionWithReceipt]) -> Self {
        // Override pre-v0.13.2 transaction hash computation
        let starknet_version = StarknetVersion::max(ctx.protocol_version, StarknetVersion::V0_13_2);

        // Verify transaction hashes. Also compute the (hash with signature, receipt hash).
        let tx_hashes_with_signature_and_receipt_hashes: Vec<_> = transactions
            .par_iter()
            .enumerate()
            .map(|(_index, tx)| {
                let got = tx.transaction.compute_hash(ctx.chain_id, starknet_version, /* is_query */ false);
                (tx.transaction.compute_hash_with_signature(got, starknet_version), tx.receipt.compute_hash())
            })
            .collect();

        let transaction_commitment = compute_transaction_commitment(
            tx_hashes_with_signature_and_receipt_hashes.iter().map(|(fst, _)| *fst),
            ctx.protocol_version,
        );

        let receipt_commitment = compute_receipt_commitment(
            tx_hashes_with_signature_and_receipt_hashes.iter().map(|(_, snd)| *snd),
            starknet_version,
        );

        Self { transaction_commitment, receipt_commitment, transaction_count: transactions.len() as u64 }
    }
}

pub struct StateDiffCommitment {
    pub state_diff_commitment: Felt,
    pub state_diff_length: u64,
}

impl StateDiffCommitment {
    pub fn compute(_ctx: &CommitmentComputationContext, state_diff: &StateDiff) -> Self {
        Self { state_diff_length: state_diff.len() as u64, state_diff_commitment: state_diff.compute_hash() }
    }
}

pub struct EventsCommitment {
    pub events_commitment: Felt,
    pub events_count: u64,
}

impl EventsCommitment {
    pub fn compute(ctx: &CommitmentComputationContext, events: &[EventWithTransactionHash]) -> Self {
        // Override pre-v0.13.2 transaction hash computation
        let starknet_version = StarknetVersion::max(ctx.protocol_version, StarknetVersion::V0_13_2);

        let event_hashes: Vec<_> =
            events.par_iter().map(|ev| ev.event.compute_hash(ev.transaction_hash, starknet_version)).collect();

        let events_commitment = compute_event_commitment(event_hashes, starknet_version);

        Self { events_commitment, events_count: events.len() as u64 }
    }
}

pub struct BlockCommitments {
    pub transaction: TransactionAndReceiptCommitment,
    pub state_diff: StateDiffCommitment,
    /// A commitment to the events produced in this block
    pub event: EventsCommitment,
}

impl BlockCommitments {
    /// Uses the rayon pool.
    pub fn compute(
        ctx: &CommitmentComputationContext,
        transactions: &[TransactionWithReceipt],
        state_diff: &StateDiff,
        events: &[EventWithTransactionHash],
    ) -> Self {
        let (transaction, (state_diff, event)) = rayon::join(
            || TransactionAndReceiptCommitment::compute(ctx, transactions),
            || rayon::join(|| StateDiffCommitment::compute(ctx, state_diff), || EventsCommitment::compute(ctx, events)),
        );
        Self { transaction, state_diff, event }
    }
}

impl PreconfirmedHeader {
    pub fn into_confirmed_header(self, commitments: BlockCommitments, global_state_root: Felt) -> Header {
        Header {
            parent_block_hash: self.parent_block_hash,
            block_number: self.block_number,
            sequencer_address: self.sequencer_address,
            block_timestamp: self.block_timestamp,
            protocol_version: self.protocol_version,
            l1_gas_price: self.l1_gas_price,
            l1_da_mode: self.l1_da_mode,
            global_state_root,
            transaction_count: commitments.transaction.transaction_count,
            transaction_commitment: commitments.transaction.transaction_commitment,
            event_count: commitments.event.events_count,
            event_commitment: commitments.event.events_commitment,
            state_diff_length: Some(commitments.state_diff.state_diff_length),
            state_diff_commitment: Some(commitments.state_diff.state_diff_commitment),
            receipt_commitment: Some(commitments.transaction.receipt_commitment),
        }
    }
}

pub fn compute_event_commitment(
    events_hashes: impl IntoIterator<Item = Felt>,
    starknet_version: StarknetVersion,
) -> Felt {
    let mut peekable = events_hashes.into_iter().peekable();
    if peekable.peek().is_none() {
        return Felt::ZERO;
    }
    if starknet_version < StarknetVersion::V0_13_2 {
        compute_merkle_root::<Pedersen>(peekable)
    } else {
        compute_merkle_root::<Poseidon>(peekable)
    }
}

pub fn compute_transaction_commitment(
    tx_hashes_with_signature: impl IntoIterator<Item = Felt>,
    starknet_version: StarknetVersion,
) -> Felt {
    if starknet_version < StarknetVersion::V0_13_2 {
        compute_merkle_root::<Pedersen>(tx_hashes_with_signature)
    } else {
        compute_merkle_root::<Poseidon>(tx_hashes_with_signature)
    }
}

pub fn compute_receipt_commitment(
    receipt_hashes: impl IntoIterator<Item = Felt>,
    _starknet_version: StarknetVersion,
) -> Felt {
    compute_merkle_root::<Poseidon>(receipt_hashes)
}

/// Compute the root hash of a list of values.
// The `HashMapDb` can't fail, so we can safely unwrap the results.
//
// perf: Note that committing changes still has the greatest performance hit
// as this is where the root hash is calculated. Due to the Merkle structure
// of Bonsai Tries, this results in a trie size that grows very rapidly with
// each new insertion. It seems that the only vector of optimization here
// would be to parallelize the tree traversal on insertion and optimize hash computation.
// It seems lambdaclass' crypto lib does not do simd hashing, we may want to look into that.
pub fn compute_merkle_root<H: StarkHash + Send + Sync>(values: impl IntoIterator<Item = Felt>) -> Felt {
    //TODO: replace the identifier by an empty slice when bonsai supports it
    const IDENTIFIER: &[u8] = b"0xinmemory";
    let config = bonsai_trie::BonsaiStorageConfig::default();
    let bonsai_db = bonsai_trie::databases::HashMapDb::<bonsai_trie::id::BasicId>::default();
    let mut bonsai_storage =
        bonsai_trie::BonsaiStorage::<_, _, H>::new(bonsai_db, config, /* max tree height */ 64);

    values.into_iter().enumerate().for_each(|(index, value)| {
        let key = BitVec::from_vec(index.to_be_bytes().to_vec()); // TODO: remove this useless allocation
        bonsai_storage.insert(IDENTIFIER, key.as_bitslice(), &value).expect("Failed to insert into bonsai storage");
    });

    let id = bonsai_trie::id::BasicIdBuilder::new().new_id();

    bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");
    bonsai_storage.root_hash(IDENTIFIER).expect("Failed to get root hash")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{BlockTimestamp, GasPrices};
    use mp_chain_config::L1DataAvailabilityMode;

    use mp_convert::ToFelt;
    use rstest::*;
    use starknet_api::{core::ChainId, felt};

    #[test]
    fn test_compute_root() {
        let values = vec![Felt::ONE, Felt::TWO, Felt::THREE];
        let root = compute_merkle_root::<Poseidon>(values);

        assert_eq!(root, Felt::from_hex_unchecked("0x3b5cc7f1292eb3847c3f902d048a7e5dc7702d1c191ccd17c2d33f797e6fc32"));
    }

    fn dummy_header() -> PreconfirmedHeader {
        PreconfirmedHeader {
            block_number: 0,
            parent_block_hash: Felt::ZERO,
            sequencer_address: Felt::ZERO,
            block_timestamp: BlockTimestamp(0),
            protocol_version: StarknetVersion::V0_13_2,
            l1_gas_price: GasPrices::default(),
            l1_da_mode: L1DataAvailabilityMode::Blob,
        }
    }

    // TODO: need more tests here.
    #[rstest]
    #[case::success(
        dummy_header(),
        1,
        felt!("0x123"),
        ChainId::Sepolia,
        false,
        felt!("0x1d34b8dac9b07ed61607e909ec2de11fc7d61d3899ebc59ca44f188ba4b7391"),
    )]
    fn test_block_hash(
        #[case] header: PreconfirmedHeader,
        #[case] _block_number: u64,
        #[case] global_state_root: Felt,
        #[case] chain_id: ChainId,
        #[case] pre_v0_13_2_override: bool,
        #[case] expected: Felt,
    ) {
        let ctx =
            CommitmentComputationContext { protocol_version: header.protocol_version, chain_id: chain_id.to_felt() };
        let commitments = BlockCommitments::compute(&ctx, &[], &StateDiff::default(), &[]);

        let closed_header = header.into_confirmed_header(commitments, global_state_root);
        let block_hash = closed_header.compute_hash(chain_id.to_felt(), pre_v0_13_2_override);
        assert_eq!(expected, block_hash);
    }
}

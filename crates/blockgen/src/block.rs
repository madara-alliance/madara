use dc_sync::commitments::{memory_event_commitment, memory_receipt_commitment, memory_transaction_commitment};
use dp_block::{DeoxysBlock, DeoxysBlockInfo, DeoxysBlockInner, Header, StarknetVersion};
use dp_receipt::{
    DeclareTransactionReceipt, DeployAccountTransactionReceipt, DeployTransactionReceipt, InvokeTransactionReceipt,
    L1HandlerTransactionReceipt, TransactionReceipt,
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use starknet_types_core::felt::Felt;

use crate::{
    transaction::{dummy_transaction, TxType, TxTypeIterator},
    BlockState,
};

impl BlockState {
    pub fn next_block(
        &mut self,
        seed: u64,
        nb_txs: usize,
        starknet_version: StarknetVersion,
        timestamp: u64,
    ) -> DeoxysBlock {
        let state_diff = self.next_state(seed);

        let block_inner = dummy_block_inner(seed, nb_txs);
        let (transaction_commitment, tx_hashes) =
            memory_transaction_commitment(&block_inner.transactions, Felt::ZERO, starknet_version, 0);
        let receipt_commitment = memory_receipt_commitment(&block_inner.receipts);
        let event_commitment = memory_event_commitment(&[], starknet_version);

        let header = Header {
            parent_block_hash: self.parent_block_hash,
            block_number: self.heigh.unwrap(),
            global_state_root: self.state_root(),
            transaction_commitment,
            sequencer_address: Felt::ZERO,
            block_timestamp: timestamp,
            transaction_count: tx_hashes.len() as u64,
            event_count: 0,
            event_commitment,
            receipt_commitment,
            state_diff_length: state_diff.len() as u64,
            state_diff_commitment: state_diff.compute_hash(),
            protocol_version: starknet_version,
            l1_gas_price: Default::default(),
            l1_da_mode: Default::default(),
        };

        let block_hash = header.compute_hash(Felt::ZERO);
        self.parent_block_hash = block_hash;

        let block_info = DeoxysBlockInfo::new(header, tx_hashes, Felt::ZERO);

        DeoxysBlock::new(block_info, block_inner)
    }
}

fn dummy_block_inner(seed: u64, nb_txs: usize) -> DeoxysBlockInner {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut tx_type_iter = TxTypeIterator::new();
    let mut block_inner = DeoxysBlockInner::default();

    for _ in 0..nb_txs {
        let tx_type = tx_type_iter.next().unwrap();
        let transaction = dummy_transaction(rng.gen(), tx_type);
        let receipt = match tx_type {
            TxType::Invoke(_) => TransactionReceipt::Invoke(InvokeTransactionReceipt::default()),
            TxType::L1Handler => TransactionReceipt::L1Handler(L1HandlerTransactionReceipt::default()),
            TxType::Declare(_) => TransactionReceipt::Declare(DeclareTransactionReceipt::default()),
            TxType::Deploy => TransactionReceipt::Deploy(DeployTransactionReceipt::default()),
            TxType::DeployAccount(_) => TransactionReceipt::DeployAccount(DeployAccountTransactionReceipt::default()),
        };
        block_inner.transactions.push(transaction);
        block_inner.receipts.push(receipt);
    }

    block_inner
}

use deoxys_runtime::opaque::DBlockT;
use log::error;
use mc_sync::l2::get_pending_block;
use mp_block::DeoxysBlock;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockId, BlockTag, EmittedEvent};
use starknet_ff::FieldElement;

use crate::errors::StarknetRpcApiError;
use crate::utils::get_block_by_block_hash;
use crate::Starknet;

impl<A: ChainApi, BE, G, C, P, H> Starknet<A, BE, G, C, P, H>
where
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    BE: Backend<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    /// Helper function to get Starknet block details
    ///
    /// # Arguments
    ///
    /// * `block_id` - The Starknet block id
    ///
    /// # Returns
    ///
    /// * `(transaction_receipts: Vec<TransactionReceiptWrapper>, block: Block)` - A tuple of the
    ///   block transaction receipts with events in block_id and an instance of Block
    pub fn get_block_events(&self, block_id: BlockId) -> Result<Vec<EmittedEvent>, StarknetRpcApiError> {
        let starknet_block = match block_id {
            BlockId::Number(n) => self.get_block_by_number(n),
            BlockId::Tag(tag) => self.get_block_by_tag(tag),
            BlockId::Hash(hash) => self.get_block_by_hash(hash),
        }?;

        let txs_hashes = if block_id == BlockId::Tag(BlockTag::Pending) {
            self.get_pending_txs_hashes(&starknet_block)?
        } else {
            self.get_block_txs_hashes(&starknet_block)?
        };

        let tx_hash_and_events: Vec<(Felt252Wrapper, _)> = starknet_block
            .events()
            .iter()
            .flat_map(|ordered_event| {
                let tx_hash = txs_hashes[ordered_event.index() as usize];
                ordered_event.events().iter().map(move |events| (tx_hash.into(), events.clone()))
            })
            .collect();

        let (block_hash, block_number) = if block_id == BlockId::Tag(BlockTag::Pending) {
            (None, None)
        } else {
            (Some(starknet_block.header().hash::<H>().0), Some(starknet_block.header().block_number))
        };
        let emitted_events = tx_hash_and_events
            .into_iter()
            .map(|(tx_hash, event)| EmittedEvent {
                from_address: Felt252Wrapper::from(event.from_address).0,
                keys: event.content.keys.into_iter().map(|felt| Felt252Wrapper::from(felt).0).collect(),
                data: event.content.data.0.into_iter().map(|felt| Felt252Wrapper::from(felt).0).collect(),
                block_hash,
                block_number,
                transaction_hash: tx_hash.0,
            })
            .collect();

        Ok(emitted_events)
    }

    fn get_block_txs_hashes(&self, starknet_block: &DeoxysBlock) -> Result<Vec<FieldElement>, StarknetRpcApiError> {
        let block_hash = starknet_block.header().hash::<H>();
        let chain_id = self.chain_id().unwrap();

        // get txs hashes from cache or compute them
        let block_txs_hashes: Vec<_> = if let Some(tx_hashes) = self.get_cached_transaction_hashes(block_hash.into()) {
            tx_hashes
                .into_iter()
                .map(|h| {
                    Felt252Wrapper::try_from(h)
                        .map(|f| f.0)
                        .map_err(|e| {
                            error!("'{e}'");
                            StarknetRpcApiError::InternalServerError
                        })
                        .unwrap()
                })
                .collect()
        } else {
            starknet_block
                .transactions_hashes::<H>(chain_id.0.into(), Some(starknet_block.header().block_number))
                .map(|tx_hash| FieldElement::from(Felt252Wrapper::from(tx_hash)))
                .collect()
        };
        Ok(block_txs_hashes)
    }

    fn get_pending_txs_hashes(&self, starknet_block: &DeoxysBlock) -> Result<Vec<FieldElement>, StarknetRpcApiError> {
        let chain_id = self.chain_id().unwrap();
        let tx_hashes: Vec<FieldElement> = starknet_block
            .transactions_hashes::<H>(chain_id.0.into(), None)
            .map(|tx_hash| FieldElement::from(Felt252Wrapper::from(tx_hash)))
            .collect();
        Ok(tx_hashes)
    }

    fn get_block_by_number(&self, block_number: u64) -> Result<DeoxysBlock, StarknetRpcApiError> {
        let substrate_block_hash =
            self.substrate_block_hash_from_starknet_block(BlockId::Number(block_number)).map_err(|e| {
                error!("'{e}'");
                StarknetRpcApiError::BlockNotFound
            })?;

        let starknet_block = get_block_by_block_hash(self.client.as_ref(), substrate_block_hash).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;
        Ok(starknet_block)
    }

    fn get_block_by_hash(&self, block_hash: FieldElement) -> Result<DeoxysBlock, StarknetRpcApiError> {
        let substrate_block_hash =
            self.substrate_block_hash_from_starknet_block(BlockId::Hash(block_hash)).map_err(|e| {
                error!("'{e}'");
                StarknetRpcApiError::BlockNotFound
            })?;

        let starknet_block = get_block_by_block_hash(self.client.as_ref(), substrate_block_hash).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;
        Ok(starknet_block)
    }

    fn get_block_by_tag(&self, block_tag: BlockTag) -> Result<DeoxysBlock, StarknetRpcApiError> {
        if block_tag == BlockTag::Latest {
            self.get_block_by_number(
                self.substrate_block_number_from_starknet_block(BlockId::Tag(BlockTag::Latest)).map_err(|e| {
                    error!("'{e}'");
                    StarknetRpcApiError::BlockNotFound
                })?,
            )
        } else {
            match get_pending_block() {
                Some(block) => Ok(block),
                _ => Err(StarknetRpcApiError::BlockNotFound),
            }
        }
    }
}

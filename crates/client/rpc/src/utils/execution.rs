use std::sync::Arc;

use anyhow::Result;
use blockifier::context::TransactionContext;
use blockifier::context::{BlockContext, ChainInfo, FeeTokenAddresses};
use blockifier::execution::entry_point::{CallEntryPoint, CallType, EntryPointExecutionContext};
use blockifier::fee::gas_usage::estimate_minimal_gas_vector;
use blockifier::state::cached_state::{CachedState, GlobalContractCache};
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::objects::{
    DeprecatedTransactionInfo, HasRelatedFeeType, TransactionExecutionInfo, TransactionInfo,
};
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::{ExecutableTransaction, L1HandlerTransaction};
use blockifier::versioned_constants::VersionedConstants;
use dc_db::db_block_id::DbBlockId;
use dc_db::storage_handler::DeoxysStorageError;
use dp_block::header::{BLOCKIFIER_VERSIONED_CONSTANTS_0_13_0, BLOCKIFIER_VERSIONED_CONSTANTS_0_13_1};
use dp_block::{DeoxysMaybePendingBlockInfo, StarknetVersion};
use dp_convert::ToFelt;
use dp_convert::ToStarkFelt;
use dp_transactions::getters::Hash;
use starknet_api::block::{BlockNumber, BlockTimestamp};
use starknet_api::core::EntryPointSelector;
use starknet_api::core::{ChainId, ContractAddress};
use starknet_api::data_availability::L1DataAvailabilityMode;
use starknet_api::deprecated_contract_class::EntryPointType;
use starknet_api::transaction::Calldata;
use starknet_api::transaction::TransactionHash;
use starknet_core::types::{FeeEstimate, PriceUnit};
use starknet_types_core::felt::Felt;

use super::blockifier_state_adapter::BlockifierStateAdapter;
use crate::errors::StarknetRpcApiError;
use crate::utils::ResultExt;
use crate::Starknet;

// 0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7
pub const ETH_TOKEN_ADDR: Felt =
    Felt::from_raw([418961398025637529, 17240401758547432026, 17839402928228694863, 4380532846569209554]);

// 0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d
pub const STRK_TOKEN_ADDR: Felt =
    Felt::from_raw([468300854463065062, 5134018303144032807, 1325769094487018516, 16432072983745651214]);

#[derive(thiserror::Error, Debug)]
pub enum BlockContextError {
    #[error("Unsupported protocol version")]
    UnsupportedProtocolVersion,
    #[error("{0:#}")]
    Reexecution(#[from] TxReexecError),
    #[error("{0:#}")]
    FeeEstimation(#[from] TxFeeEstimationError),
    #[error("{0:#}")]
    MessageFeeEstimation(#[from] MessageFeeEstimationError),
    #[error("{0:#}")]
    CallContract(#[from] CallContractError),
    #[error("Storage error: {0:#}")]
    Storage(#[from] DeoxysStorageError),
}

impl From<BlockContextError> for StarknetRpcApiError {
    fn from(value: BlockContextError) -> Self {
        match value {
            BlockContextError::UnsupportedProtocolVersion => StarknetRpcApiError::UnsupportedTxnVersion,
            BlockContextError::Reexecution(err) => {
                Err::<(), _>(err).or_internal_server_error("Reexecuting transaction").unwrap_err()
            }
            BlockContextError::Storage(err) => {
                Err::<(), _>(err).or_internal_server_error("Creating block context").unwrap_err()
            }
        }
    }
}

pub struct ExecutionContext {
    block_context: BlockContext,
    db_id: DbBlockId,
}

impl Starknet {
    pub fn block_context(
        &self,
        block_info: &DeoxysMaybePendingBlockInfo,
    ) -> Result<ExecutionContext, BlockContextError> {
        let (db_id, protocol_version, block_number, block_timestamp, sequencer_address, l1_gas_price, l1_da_mode) =
            match block_info {
                DeoxysMaybePendingBlockInfo::Pending(block) => (
                    DbBlockId::Pending,
                    block.header.protocol_version,
                    self.backend.get_latest_block_n()?.map(|el| el + 1).unwrap_or(0), // when the block is pending, we use the latest block n + 1
                    block.header.block_timestamp,
                    block.header.sequencer_address,
                    block.header.l1_gas_price,
                    block.header.l1_da_mode,
                ),
                DeoxysMaybePendingBlockInfo::NotPending(block) => (
                    DbBlockId::BlockN(block.block_n()),
                    block.header.protocol_version,
                    block.header.block_number,
                    block.header.block_timestamp,
                    block.header.sequencer_address,
                    block.header.l1_gas_price,
                    block.header.l1_da_mode,
                ),
            };

        // safe unwrap because address is always valid and static
        let fee_token_addresses = FeeTokenAddresses {
            strk_fee_token_address: STRK_TOKEN_ADDR.to_stark_felt().try_into().unwrap(),
            eth_fee_token_address: ETH_TOKEN_ADDR.to_stark_felt().try_into().unwrap(),
        };
        let chain_id = ChainId(self.chain_id().to_hex_string());

        let versioned_constants = if protocol_version < StarknetVersion::STARKNET_VERSION_0_13_0 {
            return Err(BlockContextError::UnsupportedProtocolVersion);
        } else if protocol_version < StarknetVersion::STARKNET_VERSION_0_13_1 {
            &BLOCKIFIER_VERSIONED_CONSTANTS_0_13_0
        } else if protocol_version < StarknetVersion::STARKNET_VERSION_0_13_1_1 {
            &BLOCKIFIER_VERSIONED_CONSTANTS_0_13_1
        } else {
            VersionedConstants::latest_constants()
        };

        let chain_info = ChainInfo { chain_id, fee_token_addresses };

        let block_info = blockifier::block::BlockInfo {
            block_number: BlockNumber(block_number),
            block_timestamp: BlockTimestamp(block_timestamp),
            sequencer_address: sequencer_address.to_stark_felt().try_into().unwrap(),
            gas_prices: (&l1_gas_price).into(),
            // TODO: Verify if this is correct
            use_kzg_da: l1_da_mode == L1DataAvailabilityMode::Blob,
        };

        Ok(ExecutionContext {
            block_context: BlockContext::new_unchecked(&block_info, &chain_info, versioned_constants),
            db_id,
        })
    }
}

#[derive(thiserror::Error, Debug)]
#[error("Reexecuting tx {hash:#} (index {index}) on top of {block_n}: {err:#}")]
pub struct TxReexecError {
    block_n: DbBlockId,
    hash: TransactionHash,
    index: usize,
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Estimating fee for tx index {index} on top of {block_n}: {err:#}")]
pub struct TxFeeEstimationError {
    block_n: DbBlockId,
    index: usize,
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Estimating message fee on top of {block_n}: {err:#}")]
pub struct MessageFeeEstimationError {
    block_n: DbBlockId,
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Calling contract on top of {block_n:#}: {err:#}")]
pub struct CallContractError {
    block_n: DbBlockId,
    contract: ContractAddress,
    err: TransactionExecutionError,
}

impl ExecutionContext {
    pub fn init_cached_state(&self, starknet: &Starknet) -> CachedState<BlockifierStateAdapter> {
        let block_number = self.block_context.block_info().block_number.0;
        let prev_block = block_number.checked_sub(1); // todo? handle genesis correctly
        CachedState::new(
            BlockifierStateAdapter::new(Arc::clone(&starknet.backend), prev_block),
            GlobalContractCache::new(16),
        )
    }

    pub fn re_execute_transactions(
        &self,
        starknet: &Starknet,
        transactions_before: impl IntoIterator<Item = Transaction>,
        transactions_to_trace: impl IntoIterator<Item = Transaction>,
    ) -> Result<Vec<TransactionExecutionInfo>, BlockContextError> {
        let charge_fee = self.block_context.block_info().gas_prices.eth_l1_gas_price.get() != 1;
        let mut cached_state = self.init_cached_state(starknet);

        let mut executed_prev = 0;
        for (index, tx) in transactions_before.into_iter().enumerate() {
            let tx_hash = tx.tx_hash();
            log::debug!("reexecuting {tx_hash:?}");
            tx.execute(&mut cached_state, &self.block_context, charge_fee, true).map_err(|err| TxReexecError {
                block_n: self.db_id,
                hash: tx_hash.unwrap_or_default(),
                index,
                err,
            })?;
            executed_prev += 1;
        }

        let transactions_exec_infos = transactions_to_trace
            .into_iter()
            .enumerate()
            .map(|(index, tx)| {
                let tx_hash = tx.tx_hash();
                log::debug!("reexecuting {tx_hash:?} (trace)");
                tx.execute(&mut cached_state, &self.block_context, charge_fee, true).map_err(|err| TxReexecError {
                    block_n: self.db_id,
                    hash: tx_hash.unwrap_or_default(),
                    index: executed_prev + index,
                    err,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(transactions_exec_infos)
    }

    /// Call a smart contract function.
    pub fn call_contract(
        &self,
        starknet: &Starknet,
        contract: &ContractAddress,
        function_selector: EntryPointSelector,
        calldata: Calldata,
    ) -> Result<Vec<Felt>, BlockContextError> {
        log::debug!("calling contract {contract:?}");

        let make_err = |err| CallContractError { block_n: self.db_id, contract: *contract, err };

        // Get class hash
        let class_hash = starknet.backend.get_contract_class_hash_at(&self.db_id, contract)?;

        let entrypoint = CallEntryPoint {
            class_hash,
            code_address: None,
            entry_point_type: EntryPointType::External,
            entry_point_selector: function_selector,
            calldata,
            storage_address: *contract,
            caller_address: ContractAddress::default(),
            call_type: CallType::Call,
            initial_gas: VersionedConstants::latest_constants().tx_initial_gas(),
        };

        let mut resources = cairo_vm::vm::runners::cairo_runner::ExecutionResources::default();
        let mut entry_point_execution_context = EntryPointExecutionContext::new_invoke(
            Arc::new(TransactionContext {
                block_context: self.block_context.clone(),
                tx_info: TransactionInfo::Deprecated(DeprecatedTransactionInfo::default()),
            }),
            false,
        )
        .map_err(make_err)?;

        let res = entrypoint
            .execute(
                &mut BlockifierStateAdapter::new(
                    Arc::clone(&starknet.backend),
                    Some(self.block_context.block_info().block_number.0),
                ),
                &mut resources,
                &mut entry_point_execution_context,
            )
            .map_err(TransactionExecutionError::ExecutionError)
            .map_err(make_err)?;

        log::debug!("successfully called contract {contract:?}");
        let result = res.execution.retdata.0.iter().map(|x| x.to_felt()).collect();
        Ok(result)
    }

    pub fn estimate_fee(
        &self,
        starknet: &Starknet,
        transactions: impl IntoIterator<Item = AccountTransaction>,
        validate: bool,
    ) -> Result<Vec<FeeEstimate>, BlockContextError> {
        transactions
            .into_iter()
            .enumerate()
            .map(|(tx_i, transaction)| {
                log::debug!("estimate_fee: executing tx index {tx_i:?}");

                let make_err = |err| TxFeeEstimationError { block_n: self.db_id, index: tx_i, err };

                let mut cached_state = self.init_cached_state(starknet);

                let fee_type = transaction.fee_type();

                let gas_price = self.block_context.block_info().gas_prices.get_gas_price_by_fee_type(&fee_type).get();
                let data_gas_price =
                    self.block_context.block_info().gas_prices.get_data_gas_price_by_fee_type(&fee_type).get();
                let unit = match fee_type {
                    blockifier::transaction::objects::FeeType::Strk => PriceUnit::Fri,
                    blockifier::transaction::objects::FeeType::Eth => PriceUnit::Wei,
                };

                let minimal_l1_gas_amount_vector = estimate_minimal_gas_vector(&self.block_context, &transaction)
                    .map_err(TransactionExecutionError::TransactionPreValidationError)
                    .map_err(make_err)?;

                let mut tx_info =
                    transaction.execute(&mut cached_state, &self.block_context, false, validate).map_err(make_err)?;

                if tx_info.actual_fee.0 == 0 {
                    tx_info.actual_fee = blockifier::fee::fee_utils::calculate_tx_fee(
                        &tx_info.actual_resources,
                        &self.block_context,
                        &fee_type,
                    )
                    .map_err(TransactionExecutionError::TransactionFeeError)
                    .map_err(make_err)?;
                }

                let data_gas_consumed = tx_info.da_gas.l1_data_gas;
                let data_gas_fee = data_gas_consumed.saturating_mul(data_gas_price);
                let gas_consumed = tx_info.actual_fee.0.saturating_sub(data_gas_fee) / gas_price.max(1);

                let minimal_gas_consumed = minimal_l1_gas_amount_vector.l1_gas;
                let minimal_data_gas_consumed = minimal_l1_gas_amount_vector.l1_data_gas;

                let gas_consumed = gas_consumed.max(minimal_gas_consumed);
                let data_gas_consumed = data_gas_consumed.max(minimal_data_gas_consumed);
                let overall_fee = gas_consumed
                    .saturating_mul(gas_price)
                    .saturating_add(data_gas_consumed.saturating_mul(data_gas_price));

                let fee_estimate = FeeEstimate {
                    gas_consumed: gas_consumed.into(),
                    gas_price: gas_price.into(),
                    data_gas_consumed: data_gas_consumed.into(),
                    data_gas_price: data_gas_price.into(),
                    overall_fee: overall_fee.into(),
                    unit,
                };

                Ok(fee_estimate)
            })
            .collect::<Result<Vec<_>, _>>()
    }

    pub fn estimate_message_fee(
        &self,
        starknet: &Starknet,
        message: L1HandlerTransaction,
    ) -> Result<FeeEstimate, BlockContextError> {
        let mut cached_state = self.init_cached_state(starknet);

        let unit = match message.fee_type() {
            blockifier::transaction::objects::FeeType::Strk => PriceUnit::Fri,
            blockifier::transaction::objects::FeeType::Eth => PriceUnit::Wei,
        };
        let tx_execution_infos = message
            .execute(&mut cached_state, &self.block_context, true, true)
            .map_err(|err| MessageFeeEstimationError { block_n: self.db_id, err })?;

        // TODO: implement this
        // if !tx_execution_infos.is_reverted() {}

        // TODO: implement this
        // if !tx_execution_infos.is_reverted() {}

        let fee = FeeEstimate {
            gas_consumed: Felt::from(
                tx_execution_infos.actual_resources.0.get("l1_gas_usage").cloned().unwrap_or_default(),
            ),
            gas_price: Felt::ZERO,
            data_gas_consumed: tx_execution_infos.da_gas.l1_data_gas.into(),
            data_gas_price: Felt::ZERO,
            overall_fee: tx_execution_infos.actual_fee.0.into(),
            unit,
        };
        Ok(fee)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eth_token_addr() {
        let eth_token_addr =
            Felt::from_hex("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7").unwrap();
        assert_eq!(ETH_TOKEN_ADDR, eth_token_addr);
    }

    #[test]
    fn test_strk_token_addr() {
        let strk_token_addr =
            Felt::from_hex("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d").unwrap();
        assert_eq!(STRK_TOKEN_ADDR, strk_token_addr);
    }
}

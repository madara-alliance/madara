//! Rust execution trace info types and builders.
//!
//! These types mirror the structure needed for RPC trace serialization,
//! without depending on RPC crates.

use serde::Serialize;
use starknet_types_core::felt::Felt;
use std::collections::HashMap;

use crate::contracts::account::functions::parse_calls;
use crate::contracts::paradex::{oracle, paraclear as paraclear_contract};
use crate::contracts::paradex_codegen::{paraclear_layout, token_component_layout};
use crate::gas::GasVector;
use crate::storage::{function_selector, short_string_to_felt};
use crate::types::{CallExecutionResult, ContractAddress, ExecutionResult, StateDiff};
use crate::StateReader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RustCallType {
    Regular,
    Delegate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RustEntryPointType {
    External,
    Constructor,
    L1Handler,
}

#[derive(Debug, Clone, Serialize)]
pub struct RustFunctionCall {
    pub contract_address: Felt,
    pub entry_point_selector: Felt,
    pub calldata: Vec<Felt>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RustCallInfo {
    pub function_call: RustFunctionCall,
    pub call_type: RustCallType,
    pub caller_address: Felt,
    pub class_hash: Felt,
    pub entry_point_type: RustEntryPointType,
    pub execution: CallExecutionResult,
    pub inner_calls: Vec<RustCallInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RustTransactionReceipt {
    pub actual_fee: u128,
    pub gas_consumed: GasVector,
}

#[derive(Debug, Clone, Serialize)]
pub struct RustTransactionExecutionInfo {
    pub validate_call_info: Option<RustCallInfo>,
    pub execute_call_info: Option<RustCallInfo>,
    pub fee_transfer_call_info: Option<RustCallInfo>,
    pub revert_error: Option<String>,
    pub receipt: RustTransactionReceipt,
    pub state_diff: StateDiff,
}

fn class_hash_cached<S: StateReader>(
    state: &S,
    cache: &mut HashMap<ContractAddress, Felt>,
    address: ContractAddress,
) -> Felt {
    if let Some(value) = cache.get(&address) {
        return *value;
    }
    let value = state.get_class_hash_at(address).ok().flatten().unwrap_or(Felt::ZERO);
    cache.insert(address, value);
    value
}

pub fn build_transaction_execution_info<S: StateReader>(
    state: &S,
    sender_address: Felt,
    account_class_hash: Felt,
    calldata: &[Felt],
    execute_result: &ExecutionResult,
    validate_call_info: Option<CallExecutionResult>,
    fee_transfer_call_info: Option<CallExecutionResult>,
    fee_amount: u128,
    fee_token_address: Felt,
    sequencer_address: Felt,
    gas_consumed: GasVector,
) -> RustTransactionExecutionInfo {
    let execute_selector = function_selector("__execute__");
    let validate_selector = function_selector("__validate__");
    let fee_selector = function_selector("transfer");
    let settle_trade_selector = function_selector("settle_trade_v3");

    let mut class_hash_cache = HashMap::new();
    let mut inner_calls = Vec::new();
    let mut attach_events = execute_result.call_result.events.clone();
    let mut attach_messages = execute_result.call_result.l2_to_l1_messages.clone();

    if let Ok(parsed_calls) = parse_calls(calldata) {
        inner_calls = Vec::with_capacity(parsed_calls.len());
        for (idx, call) in parsed_calls.into_iter().enumerate() {
            let class_hash = class_hash_cached(state, &mut class_hash_cache, call.to);

            let events = if idx == 0 { std::mem::take(&mut attach_events) } else { Vec::new() };
            let messages = if idx == 0 { std::mem::take(&mut attach_messages) } else { Vec::new() };
            let mut result = if idx == 0 { execute_result.call_result.retdata.clone() } else { Vec::new() };

            let mut nested_calls = Vec::new();
            if call.selector == settle_trade_selector {
                let mut markets = Vec::new();
                let push_unique_market = |markets: &mut Vec<Felt>, market: Felt| {
                    if market != Felt::ZERO && !markets.contains(&market) {
                        markets.push(market);
                    }
                };

                let trade = paraclear_contract::extract_trade_request_v3(&call.calldata).ok();
                if let Some(trade) = trade.as_ref() {
                    let transfer_registry = state
                        .get_storage_at(call.to, *token_component_layout::PARACLEAR_TRANSFER_REGISTRY_BASE)
                        .ok()
                        .unwrap_or(Felt::ZERO);
                    if transfer_registry != Felt::ZERO {
                        let selector = function_selector("decrement_pending_trade");
                        let registry_class_hash =
                            class_hash_cached(state, &mut class_hash_cache, ContractAddress(transfer_registry));
                        nested_calls.push(RustCallInfo {
                            function_call: RustFunctionCall {
                                calldata: vec![
                                    trade.id,
                                    trade.traded_at,
                                    trade.maker_order.account.0,
                                    trade.taker_order.account.0,
                                ],
                                contract_address: transfer_registry,
                                entry_point_selector: selector,
                            },
                            call_type: RustCallType::Regular,
                            caller_address: call.to.0,
                            class_hash: registry_class_hash,
                            entry_point_type: RustEntryPointType::External,
                            execution: CallExecutionResult::default(),
                            inner_calls: Vec::new(),
                        });
                    }
                    if let Ok(list) = paraclear_contract::extract_account_perp_markets_for_trace(
                        state,
                        call.to,
                        trade.maker_order.account,
                    ) {
                        for market in list {
                            push_unique_market(&mut markets, market);
                        }
                    }
                    if let Ok(list) = paraclear_contract::extract_account_perp_markets_for_trace(
                        state,
                        call.to,
                        trade.taker_order.account,
                    ) {
                        for market in list {
                            push_unique_market(&mut markets, market);
                        }
                    }
                    if markets.is_empty() {
                        push_unique_market(&mut markets, trade.maker_order.market);
                        push_unique_market(&mut markets, trade.taker_order.market);
                    }
                } else if let Ok(list) = paraclear_contract::extract_trade_markets_v3(&call.calldata) {
                    for market in list {
                        push_unique_market(&mut markets, market);
                    }
                }

                if let Ok(oracle_addr) =
                    state.get_storage_at(call.to, *paraclear_layout::PARACLEAR_ORACLE_CONTRACT_ADDRESS_BASE)
                {
                    if oracle_addr != Felt::ZERO {
                        let selector = function_selector("get_values_with_funding_indices");
                        let oracle_class_hash =
                            class_hash_cached(state, &mut class_hash_cache, ContractAddress(oracle_addr));
                        let settlement_key = short_string_to_felt("USDC");

                        let mut markets_exec: Option<CallExecutionResult> = None;
                        let mut settlement_exec: Option<CallExecutionResult> = None;
                        let mut call_oracle = |calldata: Vec<Felt>, cache: &mut Option<CallExecutionResult>| {
                            let execution = if let Some(cached) = cache.as_ref() {
                                cached.clone()
                            } else {
                                let oracle_result =
                                    oracle::execute(state, ContractAddress(oracle_addr), selector, &calldata, call.to)
                                        .ok();
                                let result =
                                    oracle_result.map(|r| r.call_result).unwrap_or_else(CallExecutionResult::default);
                                *cache = Some(result.clone());
                                result
                            };

                            nested_calls.push(RustCallInfo {
                                function_call: RustFunctionCall {
                                    calldata,
                                    contract_address: oracle_addr,
                                    entry_point_selector: selector,
                                },
                                call_type: RustCallType::Regular,
                                caller_address: call.to.0,
                                class_hash: oracle_class_hash,
                                entry_point_type: RustEntryPointType::External,
                                execution,
                                inner_calls: Vec::new(),
                            });
                        };

                        let mut markets_calldata = Vec::with_capacity(1 + markets.len());
                        markets_calldata.push(Felt::from(markets.len() as u64));
                        markets_calldata.extend(markets.iter().copied());
                        let settlement_calldata = vec![Felt::from(1u64), settlement_key];

                        call_oracle(markets_calldata.clone(), &mut markets_exec);
                        call_oracle(settlement_calldata.clone(), &mut settlement_exec);
                        call_oracle(markets_calldata, &mut markets_exec);
                        call_oracle(settlement_calldata, &mut settlement_exec);
                    }
                }
            }

            if call.selector == settle_trade_selector && result.is_empty() {
                result = vec![Felt::ONE];
            }

            inner_calls.push(RustCallInfo {
                function_call: RustFunctionCall {
                    calldata: call.calldata,
                    contract_address: call.to.0,
                    entry_point_selector: call.selector,
                },
                call_type: RustCallType::Regular,
                caller_address: sender_address,
                class_hash,
                entry_point_type: RustEntryPointType::External,
                execution: CallExecutionResult {
                    retdata: result,
                    events,
                    l2_to_l1_messages: messages,
                    failed: false,
                    gas_consumed: 0,
                },
                inner_calls: nested_calls,
            });
        }
    }

    let execute_call_info = Some(RustCallInfo {
        function_call: RustFunctionCall {
            calldata: calldata.to_vec(),
            contract_address: sender_address,
            entry_point_selector: execute_selector,
        },
        call_type: RustCallType::Regular,
        caller_address: Felt::ZERO,
        class_hash: account_class_hash,
        entry_point_type: RustEntryPointType::External,
        execution: CallExecutionResult::default(),
        inner_calls,
    });

    let validate_call_info = validate_call_info.map(|info| RustCallInfo {
        function_call: RustFunctionCall {
            calldata: calldata.to_vec(),
            contract_address: sender_address,
            entry_point_selector: validate_selector,
        },
        call_type: RustCallType::Regular,
        caller_address: Felt::ZERO,
        class_hash: account_class_hash,
        entry_point_type: RustEntryPointType::External,
        execution: info,
        inner_calls: Vec::new(),
    });

    let fee_token_class_hash = class_hash_cached(state, &mut class_hash_cache, ContractAddress(fee_token_address));
    let fee_amount_low = Felt::from(fee_amount);
    let fee_amount_high = Felt::ZERO;

    let fee_transfer_call_info = fee_transfer_call_info.map(|info| RustCallInfo {
        function_call: RustFunctionCall {
            calldata: vec![sequencer_address, fee_amount_low, fee_amount_high],
            contract_address: fee_token_address,
            entry_point_selector: fee_selector,
        },
        call_type: RustCallType::Regular,
        caller_address: sender_address,
        class_hash: fee_token_class_hash,
        entry_point_type: RustEntryPointType::External,
        execution: info,
        inner_calls: Vec::new(),
    });

    RustTransactionExecutionInfo {
        validate_call_info,
        execute_call_info,
        fee_transfer_call_info,
        revert_error: execute_result.revert_error.clone(),
        receipt: RustTransactionReceipt { actual_fee: fee_amount, gas_consumed },
        state_diff: execute_result.state_diff.clone(),
    }
}

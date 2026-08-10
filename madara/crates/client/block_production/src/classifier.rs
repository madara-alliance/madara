/// T-021/T-022: Transaction classifier + stateful class hash lookup.
///
/// classify_invoke() runs 4 checks (type, sender, selector, class hash) and returns
/// a RouteDecision. routing_state_view() resolves the execution-frontier state view
/// used for class hash lookups (T-022).
use crate::util::{RouteDecision, RouteFallbackReason, RustExecRoutingConfig};
use blockifier::transaction::transaction_execution::Transaction;
use mc_db::{MadaraBackend, MadaraStateView};
use mp_convert::{Felt, ToFelt};
use starknet_api::executable_transaction::AccountTransaction as ApiAccountTransaction;
use std::sync::Arc;

/// A single call extracted from an invoke tx calldata.
pub(crate) struct ParsedCall {
    pub contract_address: Felt,
    pub selector: Felt,
}

/// T-022: Resolve the state view at the execution frontier.
///
/// Uses `internal_preconfirmed_tip` when present (tracks execution frontier ahead of the
/// externally-visible RPC tip). Falls back to `view_on_latest_confirmed()` when no
/// preconfirmed block exists yet.
///
/// Design rule (locked): do NOT use `backend.view_on_latest()` here.
pub(crate) fn routing_state_view(backend: &Arc<MadaraBackend>) -> anyhow::Result<MadaraStateView> {
    let head = backend.chain_head_state();

    if let Some(internal_tip) = head.internal_preconfirmed_tip {
        let block = backend
            .block_view_on_preconfirmed(internal_tip)
            .ok_or_else(|| anyhow::anyhow!("missing internal preconfirmed block view for block {internal_tip}"))?;
        return Ok(block.state_view());
    }

    Ok(backend.view_on_latest_confirmed())
}

/// Look up the class hash for a contract address from the given state view.
/// Returns error if the contract is not found or lookup fails.
fn class_hash_for_contract(view: &MadaraStateView, contract_address: Felt) -> anyhow::Result<Felt> {
    view.get_contract_class_hash(&contract_address)?
        .ok_or_else(|| anyhow::anyhow!("contract class hash not found for {contract_address:#x}"))
}

/// Parse the multicall __execute__ calldata format:
///   `[n_calls, to1, sel1, cd_len1, ...cd1_data..., to2, sel2, cd_len2, ...]`
///
/// Returns None on malformed input (classifier will fail-closed to Blockifier).
fn parse_multicall_calldata(calldata: &[Felt]) -> Option<Vec<ParsedCall>> {
    if calldata.is_empty() {
        return Some(vec![]);
    }
    let n_calls: usize = u64::try_from(calldata[0]).ok()? as usize;
    let mut calls = Vec::with_capacity(n_calls);
    let mut i = 1usize;
    for _ in 0..n_calls {
        // Need at least: to, selector, cd_len
        if i + 2 >= calldata.len() {
            return None;
        }
        let contract_address = calldata[i];
        let selector = calldata[i + 1];
        let cd_len: usize = u64::try_from(calldata[i + 2]).ok()? as usize;
        i += 3 + cd_len;
        calls.push(ParsedCall { contract_address, selector });
    }
    Some(calls)
}

/// Extract (sender_address, calls) from a blockifier Transaction.
/// Returns None for non-Invoke transactions.
fn extract_invoke_calls(tx: &Transaction) -> Option<(Felt, Vec<ParsedCall>)> {
    let account_tx = match tx {
        Transaction::Account(a) => &a.tx,
        Transaction::L1Handler(_) => return None,
    };

    let invoke_tx = match account_tx {
        ApiAccountTransaction::Invoke(inv) => inv,
        ApiAccountTransaction::Declare(_) | ApiAccountTransaction::DeployAccount(_) => return None,
    };

    let sender = invoke_tx.sender_address().to_felt();

    // V0: direct contract call — a single call with contract_address + entry_point_selector
    let calls = match &invoke_tx.tx {
        starknet_api::transaction::InvokeTransaction::V0(v0) => {
            vec![ParsedCall {
                contract_address: v0.contract_address.to_felt(),
                selector: v0.entry_point_selector.to_felt(),
            }]
        }
        // V1/V3: calldata is __execute__ multicall args
        _ => parse_multicall_calldata(&invoke_tx.tx.calldata().0)?,
    };

    Some((sender, calls))
}

/// T-021: Classify a transaction into RouteDecision::Rust or RouteDecision::Blockifier.
///
/// Runs the 4 checks in order:
///   1. Must be an Invoke transaction.
///   2. Sender must be in `executor_addresses` whitelist.
///   3. Every called selector must be in `supported_selectors` whitelist.
///   4. Every called contract's class hash must be in `supported_class_hashes` whitelist.
///
/// Multicall rule: if any call is unsupported after at least one supported call has been
/// seen, emit MulticallPartiallySupported (not SelectorNotSupported / ClassHashNotSupported).
///
/// Any state lookup failure fails-closed to Blockifier (ClassHashLookupFailed).
pub(crate) fn classify_invoke(
    tx: &Transaction,
    cfg: &RustExecRoutingConfig,
    backend: &Arc<MadaraBackend>,
) -> RouteDecision {
    let Some((sender, calls)) = extract_invoke_calls(tx) else {
        return RouteDecision::Blockifier(RouteFallbackReason::NotInvoke);
    };

    if !cfg.executor_addresses.contains(&sender) {
        return RouteDecision::Blockifier(RouteFallbackReason::SenderNotExecutor);
    }

    // Obtain state view once; any failure fails-closed.
    let state_view = match routing_state_view(backend) {
        Ok(v) => v,
        Err(_) => return RouteDecision::Blockifier(RouteFallbackReason::ClassHashLookupFailed),
    };

    for (supported_so_far, call) in calls.iter().enumerate() {
        if !cfg.supported_selectors.contains(&call.selector) {
            if supported_so_far > 0 {
                return RouteDecision::Blockifier(RouteFallbackReason::MulticallPartiallySupported);
            }
            return RouteDecision::Blockifier(RouteFallbackReason::SelectorNotSupported);
        }

        let class_hash = match class_hash_for_contract(&state_view, call.contract_address) {
            Ok(h) => h,
            Err(_) => return RouteDecision::Blockifier(RouteFallbackReason::ClassHashLookupFailed),
        };

        if !cfg.supported_class_hashes.contains(&class_hash) {
            if supported_so_far > 0 {
                return RouteDecision::Blockifier(RouteFallbackReason::MulticallPartiallySupported);
            }
            return RouteDecision::Blockifier(RouteFallbackReason::ClassHashNotSupported);
        }
    }

    RouteDecision::Rust
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{AdditionalTxInfo, RouteFallbackReason, RoutedBatchToExecute, RustExecRoutingConfig};
    use blockifier::transaction::account_transaction::{AccountTransaction, ExecutionFlags};
    use blockifier::transaction::transaction_execution::Transaction;
    use mp_chain_config::ChainConfig;
    use starknet_api::core::Nonce;
    use starknet_api::executable_transaction::AccountTransaction as ApiAccountTransaction;
    use starknet_api::executable_transaction::InvokeTransaction as ApiInvokeTransaction;
    use starknet_api::executable_transaction::L1HandlerTransaction as ApiL1HandlerTransaction;
    use starknet_api::transaction::fields::{Calldata, Fee};
    use starknet_api::transaction::{
        InvokeTransaction as InvokeTransactionEnum, InvokeTransactionV1, L1HandlerTransaction as L1HandlerTxApi,
        TransactionHash,
    };
    use std::collections::HashSet;

    /// Build a minimal V1 invoke transaction with the given sender and single-call calldata.
    /// Signature is empty — the classifier does not validate signatures.
    fn make_invoke_tx(sender: Felt, to: Felt, selector: Felt) -> Transaction {
        // Single-call multicall calldata: [n_calls=1, to, selector, cd_len=0]
        let calldata_felts: Vec<Felt> = vec![Felt::ONE, to, selector, Felt::ZERO];

        let invoke_inner = InvokeTransactionV1 {
            max_fee: Default::default(),
            signature: Default::default(),
            nonce: Nonce::default(),
            sender_address: sender.try_into().expect("valid sender felt"),
            calldata: Calldata(Arc::new(calldata_felts)),
        };

        let api_invoke =
            ApiInvokeTransaction { tx: InvokeTransactionEnum::V1(invoke_inner), tx_hash: TransactionHash::default() };

        Transaction::Account(AccountTransaction {
            tx: ApiAccountTransaction::Invoke(api_invoke),
            execution_flags: ExecutionFlags::default(),
        })
    }

    /// Build a minimal L1Handler transaction (non-invoke).
    fn make_l1handler_tx() -> Transaction {
        Transaction::L1Handler(ApiL1HandlerTransaction {
            tx: L1HandlerTxApi {
                version: Default::default(),
                nonce: Nonce::default(),
                contract_address: Default::default(),
                entry_point_selector: Default::default(),
                calldata: Calldata::default(),
            },
            tx_hash: TransactionHash::default(),
            paid_fee_on_l1: Fee::default(),
        })
    }

    fn make_backend() -> Arc<MadaraBackend> {
        let chain_config = Arc::new(ChainConfig::madara_test());
        MadaraBackend::open_for_testing(chain_config)
    }

    // ──────────────────────────────────────────────────────────────────
    // Classifier unit tests (T-025)
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn not_invoke_l1_handler_returns_not_invoke() {
        let tx = make_l1handler_tx();
        let cfg = RustExecRoutingConfig::default();
        let backend = make_backend();
        assert_eq!(classify_invoke(&tx, &cfg, &backend), RouteDecision::Blockifier(RouteFallbackReason::NotInvoke));
    }

    #[test]
    fn sender_not_in_executor_addresses_returns_sender_not_executor() {
        let sender = Felt::from_hex_unchecked("0xABCD");
        let tx = make_invoke_tx(sender, Felt::ONE, Felt::TWO);
        let cfg = RustExecRoutingConfig::default(); // executor_addresses empty
        let backend = make_backend();
        assert_eq!(
            classify_invoke(&tx, &cfg, &backend),
            RouteDecision::Blockifier(RouteFallbackReason::SenderNotExecutor)
        );
    }

    #[test]
    fn selector_not_in_whitelist_returns_selector_not_supported() {
        let sender = Felt::from_hex_unchecked("0xABCD");
        let selector = Felt::from_hex_unchecked("0x2222");
        let tx = make_invoke_tx(sender, Felt::ONE, selector);
        let cfg = RustExecRoutingConfig {
            executor_addresses: HashSet::from([sender]),
            supported_selectors: HashSet::new(), // selector not whitelisted
            supported_class_hashes: HashSet::new(),
            ..Default::default()
        };
        let backend = make_backend();
        assert_eq!(
            classify_invoke(&tx, &cfg, &backend),
            RouteDecision::Blockifier(RouteFallbackReason::SelectorNotSupported)
        );
    }

    #[test]
    fn class_hash_lookup_failed_when_contract_not_deployed() {
        // Selector whitelisted but contract not in empty state → ClassHashLookupFailed.
        let sender = Felt::from_hex_unchecked("0xABCD");
        let selector = Felt::from_hex_unchecked("0x2222");
        let to = Felt::from_hex_unchecked("0xDEADBEEF");
        let tx = make_invoke_tx(sender, to, selector);
        let cfg = RustExecRoutingConfig {
            executor_addresses: HashSet::from([sender]),
            supported_selectors: HashSet::from([selector]),
            supported_class_hashes: HashSet::new(),
            ..Default::default()
        };
        let backend = make_backend(); // empty: no confirmed blocks, contract not found
        assert_eq!(
            classify_invoke(&tx, &cfg, &backend),
            RouteDecision::Blockifier(RouteFallbackReason::ClassHashLookupFailed)
        );
    }

    #[test]
    fn routing_state_view_on_empty_backend_returns_ok_and_contract_not_found() {
        // Validates T-022: routing_state_view uses latest_confirmed fallback on empty backend,
        // and get_contract_class_hash returns Ok(None) for unknown contracts.
        let backend = make_backend();
        let view = routing_state_view(&backend).unwrap();
        let result = view.get_contract_class_hash(&Felt::from_hex_unchecked("0x1234")).unwrap();
        assert!(result.is_none());
    }

    // ──────────────────────────────────────────────────────────────────
    // Partition / RoutedBatchToExecute tests (T-025)
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn blockifier_only_mode_simulated_partition_all_to_blockifier() {
        // Simulates what the batcher's partitioning loop does in BlockifierOnly mode.
        let sender = Felt::from_hex_unchecked("0xABCD");
        let selector = Felt::from_hex_unchecked("0x2222");
        let mut routed = RoutedBatchToExecute::default();
        for _ in 0..3 {
            let tx = make_invoke_tx(sender, Felt::ONE, selector);
            routed.blockifier_batch.push(tx, AdditionalTxInfo::default());
        }
        assert_eq!(routed.blockifier_batch.len(), 3);
        assert_eq!(routed.rust_batch.len(), 0);
        assert_eq!(routed.total_len(), 3);
        assert!(!routed.is_empty());
    }

    #[test]
    fn mixed_mode_sender_not_executor_routes_to_blockifier_branch() {
        let sender = Felt::from_hex_unchecked("0xABCD");
        let selector = Felt::from_hex_unchecked("0x2222");
        let cfg = RustExecRoutingConfig::default(); // empty executor_addresses
        let backend = make_backend();
        let tx = make_invoke_tx(sender, Felt::ONE, selector);
        assert!(matches!(
            classify_invoke(&tx, &cfg, &backend),
            RouteDecision::Blockifier(RouteFallbackReason::SenderNotExecutor)
        ));
    }

    #[test]
    fn routed_batch_is_empty_when_both_branches_empty() {
        let routed = RoutedBatchToExecute::default();
        assert!(routed.is_empty());
        assert_eq!(routed.total_len(), 0);
    }

    // ──────────────────────────────────────────────────────────────────
    // parse_multicall_calldata unit tests
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn parse_multicall_single_call() {
        let to = Felt::from_hex_unchecked("0x1111");
        let sel = Felt::from_hex_unchecked("0x2222");
        let data = vec![Felt::ONE, to, sel, Felt::ZERO];
        let calls = parse_multicall_calldata(&data).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].contract_address, to);
        assert_eq!(calls[0].selector, sel);
    }

    #[test]
    fn parse_multicall_empty_zero_calls() {
        let data = vec![Felt::ZERO];
        let calls = parse_multicall_calldata(&data).unwrap();
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_multicall_two_calls() {
        let to1 = Felt::from_hex_unchecked("0xAA");
        let sel1 = Felt::from_hex_unchecked("0xBB");
        let to2 = Felt::from_hex_unchecked("0xCC");
        let sel2 = Felt::from_hex_unchecked("0xDD");
        let data = vec![Felt::TWO, to1, sel1, Felt::ZERO, to2, sel2, Felt::ZERO];
        let calls = parse_multicall_calldata(&data).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].contract_address, to1);
        assert_eq!(calls[0].selector, sel1);
        assert_eq!(calls[1].contract_address, to2);
        assert_eq!(calls[1].selector, sel2);
    }

    #[test]
    fn parse_multicall_truncated_calldata_returns_none() {
        // n_calls=1 but only 2 more felts (need at least 3: to, sel, cd_len)
        let data = vec![Felt::ONE, Felt::from_hex_unchecked("0xAA"), Felt::from_hex_unchecked("0xBB")];
        assert!(parse_multicall_calldata(&data).is_none());
    }
}

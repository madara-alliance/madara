use mp_block::MadaraBlockInner;
use mp_transactions::{Transaction, TransactionWithHash};
use starknet_types_core::felt::Felt;
use starknet_types_rpc::Event;
use std::fmt;

use crate::StarknetRpcApiError;

pub fn display_internal_server_error(err: impl fmt::Display) {
    tracing::error!(target: "rpc_errors", "{:#}", err);
}

#[macro_export]
macro_rules! bail_internal_server_error {
    ($msg:literal $(,)?) => {{
        $crate::utils::display_internal_server_error(anyhow::anyhow!($msg));
        return ::core::result::Result::Err($crate::StarknetRpcApiError::InternalServerError.into())
    }};
    ($err:expr $(,)?) => {
        $crate::utils::display_internal_server_error(anyhow::anyhow!($err));
        return ::core::result::Result::Err($crate::StarknetRpcApiError::InternalServerError.into())
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::utils::display_internal_server_error(anyhow::anyhow!($fmt, $($arg)*));
        return ::core::result::Result::Err($crate::StarknetRpcApiError::InternalServerError.into())
    };
}

pub trait ResultExt<T, E> {
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, StarknetRpcApiError>;
    fn or_else_internal_server_error<C: fmt::Display, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetRpcApiError>;
    fn or_contract_error<C: fmt::Display>(self, context: C) -> Result<T, StarknetRpcApiError>;
}

impl<T, E: Into<anyhow::Error>> ResultExt<T, E> for Result<T, E> {
    #[inline]
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, StarknetRpcApiError> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                display_internal_server_error(format!("{}: {:#}", context, E::into(err)));
                Err(StarknetRpcApiError::InternalServerError)
            }
        }
    }

    #[inline]
    fn or_else_internal_server_error<C: fmt::Display, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetRpcApiError> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                display_internal_server_error(format!("{}: {:#}", context_fn(), E::into(err)));
                Err(StarknetRpcApiError::InternalServerError)
            }
        }
    }

    // TODO: should this be a thing?
    #[inline]
    fn or_contract_error<C: fmt::Display>(self, context: C) -> Result<T, StarknetRpcApiError> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                tracing::error!(target: "rpc_errors", "Contract storage error: {context}: {:#}", E::into(err));
                Err(StarknetRpcApiError::ContractError)
            }
        }
    }
}

pub trait OptionExt<T> {
    fn ok_or_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static>(
        self,
        context: C,
    ) -> Result<T, StarknetRpcApiError>;
    fn ok_or_else_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetRpcApiError>;
}

impl<T> OptionExt<T> for Option<T> {
    #[inline]
    fn ok_or_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static>(
        self,
        context: C,
    ) -> Result<T, StarknetRpcApiError> {
        match self {
            Some(val) => Ok(val),
            None => {
                display_internal_server_error(anyhow::Error::msg(context));
                Err(StarknetRpcApiError::InternalServerError)
            }
        }
    }

    #[inline]
    fn ok_or_else_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetRpcApiError> {
        match self {
            Some(val) => Ok(val),
            None => {
                display_internal_server_error(anyhow::Error::msg(context_fn()));
                Err(StarknetRpcApiError::InternalServerError)
            }
        }
    }
}

/// Filters events based on the provided address and keys.
///
/// This function checks if an event matches the given address and keys.
/// If an address is provided, the event must originate from that address.
/// The event's keys must match the provided keys pattern.
///
/// # Arguments
///
/// * `event` - A reference to the event to be filtered.
/// * `address` - An optional address that the event must originate from.
/// * `keys` - An optional slice of key patterns that the event's keys must match.
///
/// # Returns
///
/// * `true` if the event matches the address and keys pattern.
/// * `false` otherwise.
#[inline]
pub fn event_match_filter(event: &Event<Felt>, address: Option<&Felt>, keys: Option<&[Vec<Felt>]>) -> bool {
    // Check if the event's address matches the provided address, if any.
    if let Some(addr) = address {
        if addr != &event.from_address {
            return false;
        }
    }

    // If keys are not provided, return true.
    if let Some(keys) = keys {
        // Check if the number of keys in the event matches the number of provided key patterns.
        if keys.len() > event.event_content.keys.len() {
            return false;
        }

        // Check if each key in the event matches the corresponding key pattern.
        // Use iterators to traverse both keys and event.event_content.keys simultaneously.
        for (pattern, key) in keys.iter().zip(event.event_content.keys.iter()) {
            if !pattern.is_empty() && !pattern.contains(key) {
                return false;
            }
        }
    }

    true
}

/// Filters pending transactions based on an optional list of sender addresses.
///
/// This function checks if a pending transaction's sender address (extracted from `TransactionWithHash`)
/// is included in the provided list of addresses. If `addresses` is:
///
/// - [`Some`] with a vector of [`Felt`], the transaction's sender address must be present
///   in that list for the function to return `true`.
/// - [`None`], the function allows all transactions and returns `true`.
///
/// # Arguments
///
/// * `addresses` - An optional vector of `Felt` representing permissible sender addresses.
/// * `transaction` - A reference to the [`TransactionWithHash`] whose underlying [`Transaction`]
///   is checked.
///
/// # Returns
///
/// * `true` if `addresses` is `None` (no filtering) or if the transaction's sender address
///   is in the provided list of addresses.
/// * `false` otherwise.
pub fn pending_tx_match_filter(addresses: &Option<Vec<Felt>>, transaction: &TransactionWithHash) -> bool {
    // Check if the event's address is contained in the provided vector of addresses
    if let Some(address_list) = addresses {
        let sender_address = match &transaction.transaction {
            Transaction::Invoke(tx) => tx.sender_address(),
            Transaction::L1Handler(tx) => &tx.contract_address,
            Transaction::Declare(tx) => tx.sender_address(),
            Transaction::Deploy(tx) => &tx.contract_address_salt,
            Transaction::DeployAccount(tx) => tx.sender_address(),
        };
        return address_list.contains(sender_address);
    }
    true
}

/// Constructs a vector of [`TransactionWithHash`] objects from the provided [`MadaraBlockInner`]
/// and filters them by sender address.
///
/// This function pairs each transaction in the given `pending_block` with its corresponding receipt
/// to create a [`TransactionWithHash`] object. It then applies [`pending_tx_match_filter`] to
/// include only transactions whose sender addresses match those in `sender_addresses` (if provided).
///
/// # Arguments
///
/// * `pending_block` - A [`MadaraBlockInner`] containing the transactions and their receipts.
/// * `sender_addresses` - An optional vector of `Felt` representing permissible sender addresses.
///    - If [`Some`], only transactions whose sender address is in this vector are included.
///    - If [`None`], all transactions are included (no filtering).
///
/// # Returns
/// A vector of [`TransactionWithHash`] objects that satisfy the filtering criteria.
pub fn get_filtered_pending_tx_with_hash(
    pending_block: MadaraBlockInner,
    sender_addresses: &Option<Vec<Felt>>,
) -> Vec<TransactionWithHash> {
    pending_block
        .transactions
        .into_iter()
        .zip(pending_block.receipts)
        .map(|(transaction, receipt)| TransactionWithHash::new(transaction, receipt.transaction_hash()))
        .filter(|tx_with_hash| pending_tx_match_filter(sender_addresses, tx_with_hash))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;
    use starknet_types_rpc::EventContent;

    #[fixture]
    fn base_event() -> Event<Felt> {
        Event {
            from_address: Felt::from_hex_unchecked("0x1234"),
            event_content: EventContent {
                data: vec![Felt::from_hex_unchecked("0x5678")],
                keys: vec![Felt::from_hex_unchecked("0x1"), Felt::from_hex_unchecked("0x2")],
            },
        }
    }

    #[fixture]
    fn matching_address() -> Felt {
        Felt::from_hex_unchecked("0x1234")
    }

    #[fixture]
    fn non_matching_address() -> Felt {
        Felt::from_hex_unchecked("0x5678")
    }

    #[fixture]
    fn matching_keys() -> Vec<Vec<Felt>> {
        vec![vec![Felt::from_hex_unchecked("0x1")], vec![Felt::from_hex_unchecked("0x2")]]
    }

    #[fixture]
    fn non_matching_keys() -> Vec<Vec<Felt>> {
        vec![vec![Felt::from_hex_unchecked("0x1")], vec![Felt::from_hex_unchecked("0x3")]]
    }

    #[rstest]
    fn test_address_and_keys_match(base_event: Event<Felt>, matching_address: Felt, matching_keys: Vec<Vec<Felt>>) {
        assert!(event_match_filter(&base_event, Some(&matching_address), Some(&matching_keys)));
    }

    #[rstest]
    fn test_address_does_not_match(base_event: Event<Felt>, non_matching_address: Felt, matching_keys: Vec<Vec<Felt>>) {
        assert!(!event_match_filter(&base_event, Some(&non_matching_address), Some(&matching_keys)));
    }

    #[rstest]
    fn test_keys_do_not_match(base_event: Event<Felt>, matching_address: Felt, non_matching_keys: Vec<Vec<Felt>>) {
        assert!(!event_match_filter(&base_event, Some(&matching_address), Some(&non_matching_keys)));
    }

    #[rstest]
    fn test_no_address_provided(base_event: Event<Felt>, matching_keys: Vec<Vec<Felt>>) {
        assert!(event_match_filter(&base_event, None, Some(&matching_keys)));
    }

    #[rstest]
    fn test_no_keys_provided(base_event: Event<Felt>, matching_address: Felt) {
        assert!(event_match_filter(&base_event, Some(&matching_address), None));
    }

    #[rstest]
    fn test_keys_with_pattern(base_event: Event<Felt>, matching_address: Felt) {
        // [0x1 | 0x2, 0x2]
        let keys = vec![
            vec![Felt::from_hex_unchecked("0x1"), Felt::from_hex_unchecked("0x2")],
            vec![Felt::from_hex_unchecked("0x2")],
        ];
        assert!(event_match_filter(&base_event, Some(&matching_address), Some(&keys)));

        // [_, 0x3 | 0x2]
        let keys = vec![vec![], vec![Felt::from_hex_unchecked("0x3"), Felt::from_hex_unchecked("0x2")]];
        assert!(event_match_filter(&base_event, Some(&matching_address), Some(&keys)));
    }
}

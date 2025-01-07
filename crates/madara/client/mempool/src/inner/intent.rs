//! Structures representing the *intent* of executing a transaction.
//!
//! Intents are structures containing essential information for the
//! indentification of a [MempoolTransaction] inside of a [NonceTxMapping].
//! Transaction intents are received, ordered by [ArrivedAtTimestamp] and
//! resolved (polled) at a later time.
//!
//! # Readiness
//!
//! Intents are categorized by readiness. A transaction intent is marked as
//! [TransactionIntentReady] if its nonce directly follows that of the contract
//! sending the transaction, else it is [TransactionIntentPending].
//!
//! Pending intents remain pending until the transaction preceding them has been
//! polled from the [Mempool], at which point they are converted to a ready
//! intent with [TransactionIntentPending::ready].
//!
//! [MempoolTransaction]: super::MempoolTransaction
//! [NonceTxMapping]: super::NonceTxMapping
//! [Mempool]: super::super::Mempool

use starknet_api::core::Nonce;
use starknet_types_core::felt::Felt;
use std::{cmp, marker::PhantomData};

#[cfg(test)]
use crate::CheckInvariants;

use super::ArrivedAtTimestamp;

#[derive(Debug)]
pub(crate) struct MarkerReady;

#[derive(Debug)]
pub(crate) struct MarkerPending;

/// A [transaction intent] which is ready to be consumed.
///
/// [transaction intent]: IntentInner
pub(crate) type TransactionIntentReady = TransactionIntent<MarkerReady>;

/// A [transaction intent] which is waiting for the [Nonce] before it to be
/// consumed by the [Mempool]. It cannot be polled until then.
///
/// [transaction intent]: TransactionIntent
/// [Mempool]: super::super::Mempool
pub(crate) type TransactionIntentPending = TransactionIntent<MarkerPending>;

impl TransactionIntentPending {
    /// Converts this [intent] to a [TransactionIntentReady] to be added to the
    /// ready intent queue in the [MempoolInner]
    ///
    /// [intent]: self
    /// [MempoolInner]: super::MempoolInner
    pub(crate) fn ready(&self) -> TransactionIntentReady {
        TransactionIntentReady {
            contract_address: self.contract_address,
            timestamp: self.timestamp,
            nonce: self.nonce,
            nonce_next: self.nonce_next,
            phantom: Default::default(),
        }
    }
}

/// An [intent] to be consumed by the [Mempool].
///
/// This struct has the same logic applied for its implementations of [Eq] and
/// [Ord] and will check [timestamp], [contract_address] and [nonce] (in that
/// oreder) for both. [nonce_next] is not considered as it should directly
/// follow from [nonce] and therefore its equality and order is implied.
///
/// # Type Safety
///
/// This struct is statically wrapped by [TransactionIntentReady] and
/// [TransactionIntentPending] to provide type safety between intent types while
/// avoiding too much code duplication.
///
/// # [Invariants]
///
/// - [nonce_next] must always be equal to [nonce] + [Felt::ONE].
///
/// [intent]: self
/// [Mempool]: super::super::Mempool
/// [timestamp]: Self::timestamp
/// [contract_address]: Self::contract_address
/// [nonce]: Self::nonce
/// [nonce_next]: Self::nonce_next
/// [Invariants]: CheckInvariants
#[derive(Debug)]
pub(crate) struct TransactionIntent<K> {
    /// The contract responsible for sending the transaction.
    pub(crate) contract_address: Felt,
    /// Time at which the transaction was received by the mempool.
    pub(crate) timestamp: ArrivedAtTimestamp,
    /// The [Nonce] of the transaction associated to this intent. We use this
    /// for retrieval purposes later on.
    pub(crate) nonce: Nonce,
    /// This is the [Nonce] of the transaction right after this one. We
    /// precompute this to avoid making calculations on a [Felt] in the hot
    /// loop, as this can be expensive.
    pub(crate) nonce_next: Nonce,
    pub(crate) phantom: PhantomData<K>,
}

#[cfg(test)]
impl<K> CheckInvariants for TransactionIntent<K> {
    fn check_invariants(&self) {
        assert_eq!(self.nonce_next, self.nonce.try_increment().unwrap());
    }
}

impl<K> PartialEq for TransactionIntent<K> {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp
            && self.contract_address == other.contract_address
            && self.nonce == other.nonce
    }
}

impl<K> Eq for TransactionIntent<K> {}

impl<K> Ord for TransactionIntent<K> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        // Important: Fallback on contract addr here.
        // There can be timestamp collisions.
        self.timestamp
            .cmp(&other.timestamp)
            .then_with(|| self.contract_address.cmp(&other.contract_address))
            .then_with(|| self.nonce.cmp(&other.nonce))
    }
}

impl<K> PartialOrd for TransactionIntent<K> {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

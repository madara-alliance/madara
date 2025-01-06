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

use starknet_types_core::felt::Felt;
use std::{cmp, time::SystemTime};

#[cfg(test)]
use crate::CheckInvariants;

use super::ArrivedAtTimestamp;

/// An [intent] which is ready to be consumed by the [Mempool].
///
/// This struct has the same logic applied for its implementations of [Eq] and
/// [Ord] and will check [timestamp], [contract_address] and [nonce] (in that
/// oreder) for both. [nonce_next] is not considered as it should directly
/// follow from [nonce] and therefore its equality and order is implied. This
/// differs from [TransactionIntentPending].
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
#[derive(Clone, Debug)]
pub(crate) struct TransactionIntentReady {
    /// The contract responsible for sending the transaction.
    pub(crate) contract_address: Felt,
    /// Time at which the transaction was received by the mempool.
    pub(crate) timestamp: ArrivedAtTimestamp,
    /// The nonce of the transaction associated to this intent. We use this for
    /// retrieval purposes later on.
    pub(crate) nonce: Felt,
    /// This is the nonce of the transaction right after this one. We precompute
    /// this to avoid making calculations on a [Felt] in the hot loop, as this
    /// can be expensive.
    pub(crate) nonce_next: Felt,
}

impl TransactionIntentReady {
    /// Creates a new [TransactionIntentPending] to be used as the key in a map.
    ///
    /// This is used to check if the next transaction for this account is
    /// available in the pending intent queue of the [MempoolInner],
    ///
    /// [MempoolInner]: super::MempoolInner
    pub(crate) fn tx_next_for_lookup(self) -> TransactionIntentPending {
        TransactionIntentPending {
            contract_address: self.contract_address,
            // We do not care about the time as it is not considered in the
            // implementation of Eq for AccountPending
            timestamp: SystemTime::UNIX_EPOCH,
            nonce: self.nonce_next,
            // Same for nonce_next
            nonce_next: Felt::ZERO,
        }
    }
}

#[cfg(test)]
impl CheckInvariants for TransactionIntentReady {
    fn check_invariants(&self) {
        assert!(self.nonce_next == self.nonce + Felt::ONE);
    }
}

impl PartialEq for TransactionIntentReady {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp
            && self.contract_address == other.contract_address
            && self.nonce == other.nonce
    }
}

impl Eq for TransactionIntentReady {}

impl Ord for TransactionIntentReady {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        // Important: Fallback on contract addr here.
        // There can be timestamp collisions.
        self.timestamp
            .cmp(&other.timestamp)
            .then_with(|| self.contract_address.cmp(&other.contract_address))
            .then_with(|| self.nonce.cmp(&other.nonce))
    }
}

impl PartialOrd for TransactionIntentReady {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A [intent] which is waiting for the nonce before it to be consumed by the
/// [Mempool]. It cannot be polled until then.
///
/// This struct does not consider [timestamp] or [nonce_next] in its
/// implementations of [Eq] or [PartialEq], which allows a
/// [TransactionIntentReady] to easily retrieve the pending intent directly
/// following it.
///
/// # [Invariants]
///
/// - [nonce_next] must always be equal to [nonce] + [Felt::ONE]
///
/// [intent]: self
/// [Mempool]: super::super::Mempool
/// [timestamp]: Self::timestamp
/// [nonce]: Self::nonce
/// [nonce_next]: Self::nonce_next
/// [Invariants]: CheckInvariants
#[derive(Clone, Debug)]
pub(crate) struct TransactionIntentPending {
    /// The contract responsible for sending the transaction.
    pub(crate) contract_address: Felt,
    /// Time at which the transaction was received by the mempool.
    pub(crate) timestamp: ArrivedAtTimestamp,
    /// The nonce of the transaction associated to this intent. We use this for
    /// retrieval purposes later on.
    pub(crate) nonce: Felt,
    /// This is the nonce of the transaction right after this one. We precompute
    /// this to avoid making calculations on a [Felt] in the hot loop, as this
    /// can be expensive.
    pub(crate) nonce_next: Felt,
}

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
        }
    }
}

#[cfg(test)]
impl CheckInvariants for TransactionIntentPending {
    fn check_invariants(&self) {
        assert!(self.nonce_next == self.nonce + Felt::ONE);
    }
}

impl PartialEq for TransactionIntentPending {
    fn eq(&self, other: &Self) -> bool {
        // We don't care about the timestamp for pending transactions: instead,
        // we want a ready transaction to be able to easily poll the pending
        // transaction at the same contract with the next nonce.
        //
        // WARN: This has the implication that later transactions with the same
        // nonce will be rejected.
        self.contract_address == other.contract_address && self.nonce == other.nonce
    }
}

impl Eq for TransactionIntentPending {}

impl Ord for TransactionIntentPending {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.contract_address.cmp(&other.contract_address).then_with(|| self.nonce.cmp(&other.nonce))
    }
}

impl PartialOrd for TransactionIntentPending {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

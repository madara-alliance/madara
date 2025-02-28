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
//! sending the transaction, else it marked as pending.
//!
//! # Pending intents
//!
//! There are two types of pending intents [TransactionIntentPendingByNonce] and
//! [TransactionIntentPendingByTimestamp]. Each pending intent contains the same
//! data but with slightly different ordering rules. This is because the
//! [MempoolInner] holds two ordered queues for pending intents:
//!
//! - One is ordered by timestamp to facilitate the removal of age-exceeded
//!   pending intents.
//!
//! - The other is ordered by nonces to be able to easily retrieve the
//!   transaction with the next nonce for a specific contract.
//!
//! > You can convert from one pending intent type to another with [by_timestamp]
//! > and [by_nonce].
//!
//! Both pending intents remain pending until the transaction preceding them has
//! been polled from the [Mempool], at which point [TransactionIntentPendingByNonce]
//! is converted to a ready intent with [TransactionIntentPendingByNonce::ready],
//! while any [TransactionIntentPendingByTimestamp] are removed from the queue.
//!
//! [MempoolTransaction]: super::MempoolTransaction
//! [NonceTxMapping]: super::NonceTxMapping
//! [MempoolInner]: super::MempoolInner
//! [Mempool]: super::super::Mempool
//! [by_timestamp]: TransactionIntentPendingByNonce::by_timestamp
//! [by_nonce]: TransactionIntentPendingByTimestamp::by_nonce

use starknet_api::core::Nonce;
use starknet_types_core::felt::Felt;
use std::{cmp, marker::PhantomData};

#[cfg(any(test, feature = "testing"))]
use crate::CheckInvariants;

use super::ArrivedAtTimestamp;

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(Clone))]
pub(crate) struct MarkerReady;

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(Clone))]
pub(crate) struct MarkerPendingByNonce;

#[derive(Debug)]
#[cfg_attr(any(test, feature = "testing"), derive(Clone))]
pub(crate) struct MarkerPendingByTimestamp;

/// A [transaction intent] which is ready to be consumed.
///
/// [transaction intent]: TransactionIntent
pub(crate) type TransactionIntentReady = TransactionIntent<MarkerReady>;

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

/// A [transaction intent] which is waiting for the [Nonce] before it to be
/// consumed by the [Mempool]. It cannot be polled until then.
///
/// [transaction intent]: TransactionIntent
/// [Mempool]: super::super::Mempool
pub(crate) type TransactionIntentPendingByNonce = TransactionIntent<MarkerPendingByNonce>;

impl TransactionIntentPendingByNonce {
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
            phantom: std::marker::PhantomData,
        }
    }

    /// Converts this [intent] to a [TransactionIntentPendingByTimestamp] to be
    /// used to remove aged pending transactions from the [MempoolInner].
    ///
    /// [intent]: self
    /// [MempoolInner]: super::MempoolInner
    pub(crate) fn by_timestamp(&self) -> TransactionIntentPendingByTimestamp {
        TransactionIntentPendingByTimestamp {
            contract_address: self.contract_address,
            timestamp: self.timestamp,
            nonce: self.nonce,
            nonce_next: self.nonce_next,
            phantom: std::marker::PhantomData,
        }
    }
}

impl Ord for TransactionIntentPendingByNonce {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        // Pending transactions are simply ordered by nonce
        self.nonce
            .cmp(&other.nonce)
            .then_with(|| self.timestamp.cmp(&other.timestamp))
            .then_with(|| self.contract_address.cmp(&other.contract_address))
    }
}

impl PartialOrd for TransactionIntentPendingByNonce {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A [pending transaction intent] which is ordered by timestamp. This is
/// necessary to be able to remove pending transactions which have grown too old
/// in the [Mempool].
///
/// [pending transaction intent]: TransactionIntentPendingByNonce
/// [Mempool]: super::super::Mempool
pub(crate) type TransactionIntentPendingByTimestamp = TransactionIntent<MarkerPendingByTimestamp>;

impl TransactionIntentPendingByTimestamp {
    pub(crate) fn by_nonce(self) -> TransactionIntentPendingByNonce {
        TransactionIntentPendingByNonce {
            contract_address: self.contract_address,
            timestamp: self.timestamp,
            nonce: self.nonce,
            nonce_next: self.nonce_next,
            phantom: PhantomData,
        }
    }
}

impl Ord for TransactionIntentPendingByTimestamp {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        // Important: Fallback on contract addr here.
        // There can be timestamp collisions.
        self.timestamp
            .cmp(&other.timestamp)
            .then_with(|| self.contract_address.cmp(&other.contract_address))
            .then_with(|| self.nonce.cmp(&other.nonce))
    }
}

impl PartialOrd for TransactionIntentPendingByTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// An [intent] to be consumed by the [Mempool].
///
/// This data struct will check [timestamp], [contract_address] and [nonce]
/// (in that order) for equality. [nonce_next] is not considered as it should
/// directly follow from [nonce] and therefore its equality and order is implied.
///
/// # Type Safety
///
/// This struct is statically wrapped by [TransactionIntentReady],
/// [TransactionIntentPendingByNonce] and [TransactionIntentPendingByTimestamp]
/// to provide type safety between intent types while avoiding too much code
/// duplication.
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
#[cfg_attr(any(test, feature = "testing"), derive(Clone))]
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

#[cfg(any(test, feature = "testing"))]
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

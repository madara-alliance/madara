use mc_p2p::{P2pCommands, PeerId};
use rand::{thread_rng, Rng};
use std::cmp;
use std::collections::{hash_map, BTreeSet, HashMap, HashSet};
use std::num::Saturating;
use std::ops::Deref;
use std::sync::Arc;
use tokio::time::{Duration, Instant};

// TODO: add bandwidth metric
#[derive(Default, Debug, Clone)]
struct PeerStats {
    // prioritize peers that are behaving correctly
    successes: Saturating<i32>,
    // avoid peers that are behaving incorrectly
    // TODO: we may want to differenciate timeout failures and bad-data failures.
    // => we probably want to evict bad-data failures in every case.
    failures: Saturating<i32>,
    // avoid peers that are currently in use
    in_use_counter: Saturating<u32>,
    rand_additional: Saturating<i32>,
}

impl PeerStats {
    fn increment_successes(&mut self) {
        self.successes += 1;
    }
    fn increment_failures(&mut self) {
        self.failures += 1;
    }
    fn increment_in_use(&mut self) {
        self.in_use_counter += 1;
    }
    fn decrement_in_use(&mut self) {
        self.in_use_counter -= 1;
    }
    fn reroll_rand(&mut self) {
        self.rand_additional = Saturating(thread_rng().gen_range(-5..5));
    }
    fn score(&self) -> i64 {
        // it's okay to use peers that are currently in use, but we don't want to rely on only one peer all the time
        // so, we put a temporary small malus if the peer is already in use.
        // if we are using the peer a lot, we want that malus to be higher - we really don't want to spam a single peer.
        let in_use_malus = if self.in_use_counter < Saturating(16) {
            self.in_use_counter * Saturating(2)
        } else {
            self.in_use_counter * Saturating(5)
        };
        let in_use_malus = Saturating(in_use_malus.0 as i32);

        // we only count up to 20 successes, to avoid having a score go too high.
        let successes = self.successes.min(Saturating(20));

        (Saturating(-10) * self.failures + successes - in_use_malus + self.rand_additional).0.into()
    }

    fn should_evict(&self) -> bool {
        self.failures >= Saturating(5)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct PeerSortedByScore {
    peer_id: PeerId,
    score: i64,
}
impl PeerSortedByScore {
    pub fn new(peer_id: PeerId, stats: &PeerStats) -> Self {
        Self { peer_id, score: stats.score() }
    }
}

impl Ord for PeerSortedByScore {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        // other and self are swapped because we want the highest score to lowest

        // score can have collisions, so we compare by peer_id next
        other.score.cmp(&self.score).then(other.peer_id.cmp(&self.peer_id))
    }
}
impl PartialOrd for PeerSortedByScore {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// TODO: add a mode to get a random peer regardless of score.
// Invariants:
// 1) there should be a one-to-one correspondance between the `queue` and `stats_by_peer` variable.
#[derive(Default, Debug)]
struct PeerSetInner {
    queue: BTreeSet<PeerSortedByScore>,
    stats_by_peer: HashMap<PeerId, PeerStats>,
    evicted_peers_ban_deadlines: HashMap<PeerId, Instant>,
}

impl PeerSetInner {
    const EVICTION_BAN_DELAY: Duration = Duration::from_secs(3);

    fn peek_next(&self) -> Option<PeerId> {
        self.queue.first().map(|p| p.peer_id)
    }

    fn update_stats(&mut self, peer: PeerId, f: impl FnOnce(&mut PeerStats)) {
        match self.stats_by_peer.entry(peer) {
            hash_map::Entry::Occupied(mut entry) => {
                // Remove old queue entry.
                let removed = self.queue.remove(&PeerSortedByScore::new(peer, entry.get()));
                debug_assert!(removed, "Invariant 1 violated");

                // Update the stats in-place
                f(entry.get_mut());

                if entry.get().should_evict() {
                    // evict
                    entry.remove();
                    tracing::debug!("Peer Set Evicting {peer} for {:?}", Self::EVICTION_BAN_DELAY);
                    self.evicted_peers_ban_deadlines.insert(peer, Instant::now() + Self::EVICTION_BAN_DELAY);
                } else {
                    entry.get_mut().reroll_rand();

                    // Reinsert the queue entry with the new score.
                    // If insert returns true, the value is already in the queue - which would mean that the peer id is duplicated in the queue.
                    // `stats_by_peer` has PeerId as key and as such cannot have a duplicate peer id. This means that if there is a duplicated
                    // peer_id in the queue, there is not a one-to-one correspondance between the two datastructures.
                    let inserted = self.queue.insert(PeerSortedByScore::new(peer, entry.get()));
                    debug_assert!(inserted, "Invariant 1 violated");
                }
            }
            hash_map::Entry::Vacant(_entry) => {}
        }
        tracing::debug!("Peer Set Update stats for {peer}");
    }

    fn append_new_peers(&mut self, new_peers: impl IntoIterator<Item = PeerId>) {
        let now = Instant::now();
        self.evicted_peers_ban_deadlines.retain(|_, v| *v > now);

        for peer_id in new_peers.into_iter() {
            if self.evicted_peers_ban_deadlines.contains_key(&peer_id) {
                continue;
            }

            if let hash_map::Entry::Vacant(entry) = self.stats_by_peer.entry(peer_id) {
                let stats = PeerStats::default();
                self.queue.insert(PeerSortedByScore::new(peer_id, &stats));
                entry.insert(stats);
            }
        }
        tracing::debug!("Append new peers now: {:#?} peers", self.stats_by_peer.len());
    }
}

pub struct GetPeersInner {
    wait_until: Option<Instant>,
    commands: P2pCommands,
}

impl GetPeersInner {
    /// We avoid spamming get_random_peers: the start of each get_random_peers request must be separated by at least this duration.
    /// This has no effect if the get_random_peers operation takes more time to complete than this delay.
    const GET_RANDOM_PEERS_DELAY: Duration = Duration::from_millis(3000);

    pub fn new(commands: P2pCommands) -> Self {
        // We have a start-up wait until, because the p2p service may take some time to boot up.
        Self { commands, wait_until: Some(Instant::now() + Self::GET_RANDOM_PEERS_DELAY) }
    }

    pub async fn get_new_peers(&mut self) -> HashSet<PeerId> {
        let now = Instant::now();

        if let Some(inst) = self.wait_until {
            if inst > now {
                tokio::time::sleep_until(inst).await;
            }
        }
        self.wait_until = Some(now + Self::GET_RANDOM_PEERS_DELAY);

        let mut res = self.commands.get_random_peers().await;
        tracing::trace!("Got get_random_peers answer: {res:?}");
        res.remove(&self.commands.peer_id()); // remove ourselves from the response, in case we find ourselves
        if res.is_empty() {
            tracing::warn!(
                "Could not find any peer in network. Please check that your network configuration is correct."
            );
        }
        res
    }
}

// TODO: we may want to invalidate the peer list over time
// Mutex order: to statically ensure deadlocks are not possible, inner should always be locked after get_peers_mutex, if the two need to be taken at once.
pub struct PeerSet {
    // Tokio mutex: when the peer set is empty, we want to .await to get new peers
    // This is behind a mutex because we don't want to have concurrent get_more_peers requests. If there is already a request in flight, this mutex ensures we wait until that
    // request finishes before trying to get even more peers.
    get_more_peers_mutex: tokio::sync::Mutex<GetPeersInner>,
    // Std mutex: underlying datastructure, all accesses are sync
    inner: std::sync::Mutex<PeerSetInner>,
}

impl PeerSet {
    pub fn new(commands: P2pCommands) -> Self {
        Self {
            get_more_peers_mutex: tokio::sync::Mutex::new(GetPeersInner::new(commands)),
            inner: std::sync::Mutex::new(PeerSetInner::default()),
        }
    }

    /// Returns the next peer to use. If there is are no peers currently in the set,
    /// it will start a get random peers command.
    pub async fn next_peer(self: &Arc<Self>) -> anyhow::Result<PeerGuard> {
        let peer_id = self.next_peer_inner().await?;
        Ok(PeerGuard { peer_set: Some(self.clone()), peer_id })
    }

    async fn next_peer_inner(&self) -> anyhow::Result<PeerId> {
        fn next_from_set(inner: &mut PeerSetInner) -> Option<PeerId> {
            inner.peek_next().inspect(|peer| {
                // this will update the queue order, so that we can return another peer next time this function is called.
                inner.update_stats(*peer, |stats| {
                    stats.increment_in_use();
                });
            })
        }

        if let Some(peer) = next_from_set(&mut self.inner.lock().expect("Poisoned lock")) {
            return Ok(peer);
        }

        loop {
            let mut guard = self.get_more_peers_mutex.lock().await;

            // Some other task may have filled the peer set for us while we were waiting.
            if let Some(peer) = next_from_set(&mut self.inner.lock().expect("Poisoned lock")) {
                return Ok(peer);
            }

            let new_peers = guard.get_new_peers().await;
            // note: this is the only place where the two locks are taken at the same time.
            // see structure detail for lock order.
            let mut inner = self.inner.lock().expect("Poisoned lock");
            inner.append_new_peers(new_peers);

            if let Some(peer) = next_from_set(&mut inner) {
                return Ok(peer);
            }
        }
    }

    /// Signal that the peer did not follow the protocol correctly, sent bad data or timed out.
    /// We may want to avoid this peer in the future.
    fn peer_operation_error(&self, peer_id: PeerId) {
        tracing::debug!("peer_operation_error: {peer_id:?}");
        let mut inner = self.inner.lock().expect("Poisoned lock");
        inner.update_stats(peer_id, |stats| {
            stats.decrement_in_use();
            stats.increment_failures();
        })
    }

    /// Signal that the operation with the peer was successful.
    ///
    // TODO: add a bandwidth argument to allow the peer set to score and avoid being drip-fed.
    fn peer_operation_success(&self, peer_id: PeerId) {
        tracing::debug!("peer_operation_success: {peer_id:?}");
        let mut inner = self.inner.lock().expect("Poisoned lock");
        inner.update_stats(peer_id, |stats| {
            stats.decrement_in_use();
            stats.increment_successes();
        })
    }

    /// Neutral signal that the operation was dropped by our decision. No malus nor bonus.
    fn peer_operation_drop(&self, peer_id: PeerId) {
        tracing::debug!("peer_operation_drop: {peer_id:?}");
        let mut inner = self.inner.lock().expect("Poisoned lock");
        inner.update_stats(peer_id, |stats| {
            stats.decrement_in_use();
        })
    }
}

pub struct PeerGuard {
    peer_set: Option<Arc<PeerSet>>,
    peer_id: PeerId,
}

impl Deref for PeerGuard {
    type Target = PeerId;
    fn deref(&self) -> &Self::Target {
        &self.peer_id
    }
}

impl PeerGuard {
    pub fn success(mut self) {
        self.peer_set.take().expect("Peer set already taken").peer_operation_success(self.peer_id)
    }
    pub fn error(mut self) {
        self.peer_set.take().expect("Peer set already taken").peer_operation_error(self.peer_id)
    }
}

impl Drop for PeerGuard {
    // Note: we use an Option because success() and error() will still call the destructor.
    fn drop(&mut self) {
        if let Some(peer_set) = self.peer_set.take() {
            peer_set.peer_operation_drop(self.peer_id)
        }
    }
}

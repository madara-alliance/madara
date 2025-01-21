use ahash::AHasher;
use mp_receipt::Event;
use serde::{Deserialize, Deserializer, Serialize};
use starknet_types_core::felt::Felt;
use std::{collections::HashSet, fmt};

use crate::{AtomicBitStore, BitStore, BloomFilter, PreCalculatedHashes};

/// Number of hash functions used in the Bloom filter.
/// The value 7 is optimal for a false positive rate of 1% (0.01).
/// Formula used: k = -ln(p)/ln(2) ≈ 7
/// where:
/// - k is the number of hash functions
/// - p is the desired false positive rate (0.01)
/// Reference: [Bloom, B. H. (1970). "Space/Time Trade-offs in Hash Coding with Allowable Errors"](https://dl.acm.org/doi/10.1145/362686.362692)
const HASH_COUNT: u8 = 7;

/// Number of bits per element in the Bloom filter.
/// The value 9.6 is derived from the formula for optimal size:
/// m/n = -ln(p)/ln(2)² ≈ 9.6
/// where:
/// - m is the size of the filter in bits
/// - n is the number of elements
/// - p is the desired false positive rate (0.01)
const BITS_PER_ELEMENT: f64 = 9.6;

type BloomHasher = AHasher;

pub struct EventBloomWriter {
    filter: BloomFilter<BloomHasher, AtomicBitStore>,
}

impl fmt::Debug for EventBloomWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EventBloomWriter {{ size: {}, hash_count: {} }}", self.size(), HASH_COUNT)
    }
}

impl Serialize for EventBloomWriter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.filter.storage().serialize(serializer)
    }
}

impl EventBloomWriter {
    pub fn from_events<'a, I>(events: I) -> Self
    where
        I: Iterator<Item = &'a Event> + 'a,
    {
        let mut unique_keys = HashSet::new();

        for key in Self::events_to_bloom_keys(events) {
            unique_keys.insert(key);
        }

        let size = (unique_keys.len() as f64 * BITS_PER_ELEMENT).ceil() as _;

        let filter = BloomFilter::<_, AtomicBitStore>::new(size, HASH_COUNT);

        for key in unique_keys {
            filter.add(&key);
        }
        Self { filter }
    }

    pub fn size(&self) -> u64 {
        self.filter.len()
    }

    fn events_to_bloom_keys<'a, I>(events: I) -> impl Iterator<Item = [u8; 33]> + 'a
    where
        I: Iterator<Item = &'a Event> + 'a,
    {
        events.flat_map(Self::event_to_bloom_keys)
    }

    fn event_to_bloom_keys(event: &Event) -> impl Iterator<Item = [u8; 33]> + '_ {
        let from_address_key = create_bloom_key(0, &event.from_address);

        let keys = event.keys.iter().enumerate().map(|(i, key)| create_bloom_key(i as u8 + 1, key));

        std::iter::once(from_address_key).chain(keys)
    }
}

pub struct EventBloomReader {
    filter: BloomFilter<BloomHasher, BitStore>,
}

impl<'de> Deserialize<'de> for EventBloomReader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let storage = BitStore::deserialize(deserializer)?;

        Ok(Self { filter: BloomFilter::from_storage(storage, HASH_COUNT) })
    }
}

pub struct EventBloomSearcher {
    patterns: Vec<Vec<PreCalculatedHashes>>,
}

impl EventBloomSearcher {
    pub fn new(from_address: Option<&Felt>, keys: Option<&[Vec<Felt>]>) -> Self {
        let mut patterns = Vec::new();

        if let Some(from_address) = from_address {
            let from_address_key = create_bloom_key(0, from_address);
            patterns.push(vec![PreCalculatedHashes::new::<BloomHasher, _>(HASH_COUNT, &from_address_key)]);
        }

        if let Some(keys) = keys {
            for (i, key) in keys.iter().enumerate() {
                if key.is_empty() {
                    continue;
                }
                patterns.push(
                    key.iter()
                        .map(|key| create_bloom_key(i as u8 + 1, key))
                        .map(|key| PreCalculatedHashes::new::<BloomHasher, _>(HASH_COUNT, &key))
                        .collect(),
                );
            }
        }

        Self { patterns }
    }

    pub fn search(&self, filter: &EventBloomReader) -> bool {
        self.patterns.iter().all(|pattern| pattern.iter().any(|hashes| filter.filter.might_contain_hashes(hashes)))
    }
}

fn create_bloom_key(index: u8, key: &Felt) -> [u8; 33] {
    let mut bloom_key = [0u8; 33];
    bloom_key[0] = index;
    bloom_key[1..].copy_from_slice(&key.to_bytes_be());
    bloom_key
}

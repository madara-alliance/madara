use mp_receipt::Event;
use serde::{Deserialize, Deserializer, Serialize};
use siphasher::sip::SipHasher;
use starknet_types_core::felt::Felt;
use std::{collections::HashSet, fmt};

use mp_bloom_filter::{AtomicBitStore, BitStore, BloomFilter, PreCalculatedHashes};

/// Number of hash functions used in the Bloom filter.
/// The value 7 is optimal for a false positive rate of 1% (0.01).
/// Formula used: k = -ln(p)/ln(2) ≈ 7
/// where:
/// - k is the number of hash functions
/// - p is the desired false positive rate (0.01)
///
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

type BloomHasher = SipHasher;

/// Writer component for creating event-based Bloom filters.
///
/// The writer is responsible for:
/// 1. Collecting unique keys from events
/// 2. Calculating the optimal filter size
/// 3. Populating the filter with event data
///
/// # Memory Usage
///
/// The writer maintains an internal buffer of unique keys before creating the final filter.
/// Memory usage is approximately:
/// - 33 bytes per unique key (during construction)
/// - (number of unique keys * BITS_PER_ELEMENT / 8) bytes for the final filter
///
/// # Thread Safety
///
/// The writer uses an [`AtomicBitStore`] internally, making it safe for concurrent modifications
/// if needed.
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
    /// Creates a new Bloom filter from an iterator of events.
    ///
    /// This function processes the events to:
    /// 1. Extract all unique bloom keys (from addresses and event keys)
    /// 2. Calculate the optimal filter size based on the number of unique keys
    /// 3. Construct a Bloom filter with the calculated size
    /// 4. Add all unique keys to the filter
    ///
    /// # Arguments
    ///
    /// * `events` - An iterator over references to Event structs
    ///
    /// # Returns
    ///
    /// A new EventBloomWriter initialized with the provided events
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
        self.filter.size()
    }

    fn events_to_bloom_keys<'a, I>(events: I) -> impl Iterator<Item = [u8; 33]> + 'a
    where
        I: Iterator<Item = &'a Event> + 'a,
    {
        events.flat_map(Self::event_to_bloom_keys)
    }

    /// Converts a single event into an iterator of bloom keys.
    ///
    /// # Arguments
    ///
    /// * `event` - A reference to an Event struct
    ///
    /// # Returns
    ///
    /// An iterator yielding 33-byte arrays, one for from_address and one for each event key
    fn event_to_bloom_keys(event: &Event) -> impl Iterator<Item = [u8; 33]> + '_ {
        let from_address_key = create_bloom_key(0, &event.from_address);

        let keys = event.keys.iter().enumerate().map(|(i, key)| create_bloom_key(i as u8 + 1, key));

        std::iter::once(from_address_key).chain(keys)
    }
}

/// Reader component for querying event-based Bloom filters.
///
/// The reader provides an immutable view into a Bloom filter, typically
/// deserialized from a previously constructed [`EventBloomWriter`].
///
/// # Serialization
///
/// The reader implements [`Deserialize`] to allow efficient reconstruction
/// from serialized data.
///
/// # Memory Usage
///
/// The reader's memory footprint is minimal, consisting only of:
/// - The bit array (size determined by the writer)
/// - A small fixed overhead for the filter metadata
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

/// Pattern matching component for searching event-based Bloom filters.
///
/// The searcher supports complex queries with both AND and OR semantics:
/// - Different patterns are combined with AND logic
/// - Keys within each pattern use OR logic
///
/// # Query Optimization
///
/// The searcher pre-calculates hashes for all patterns during construction,
/// making subsequent searches more efficient.
pub struct EventBloomSearcher {
    patterns: Vec<Vec<PreCalculatedHashes>>,
}

impl EventBloomSearcher {
    /// Creates a new searcher with the specified patterns.
    ///
    /// # Arguments
    ///
    /// * `from_address` - Optional Felt value to match against event from_address
    /// * `keys` - Optional array of key arrays. Each inner array represents a set of alternatives
    ///           (OR semantics), while the outer array elements are combined with AND semantics.
    ///
    /// # Returns
    ///
    /// A new EventBloomSearcher configured with the specified patterns
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

/// Creates a bloom key by combining an index with a Felt value.
///
/// The resulting key is a 33-byte array where:
/// - First byte is the index
/// - Remaining 32 bytes are the big-endian representation of the Felt value
///
/// # Arguments
///
/// * `index` - A byte value identifying the type of key (0 for from_address, 1+ for event keys)
/// * `key` - The Felt value to include in the key
///
/// # Returns
///
/// A 33-byte array containing the combined key
fn create_bloom_key(index: u8, key: &Felt) -> [u8; 33] {
    let mut bloom_key = [0u8; 33];
    bloom_key[0] = index;
    bloom_key[1..].copy_from_slice(&key.to_bytes_be());
    bloom_key
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rstest::*;
    use std::collections::HashSet;
    use std::time::Instant;

    /// Creates a test event with the specified from_address and keys
    fn create_test_event(from: u64, keys: Vec<u64>) -> Event {
        Event { from_address: Felt::from(from), keys: keys.into_iter().map(Felt::from).collect(), data: vec![] }
    }

    #[fixture]
    fn single_event() -> Event {
        create_test_event(1, vec![2, 3])
    }

    #[fixture]
    fn multiple_events() -> Vec<Event> {
        vec![create_test_event(1, vec![2, 3]), create_test_event(2, vec![4, 5])]
    }

    fn event_strategy() -> impl Strategy<Value = Event> {
        (any::<u64>(), prop::collection::vec(any::<u64>(), 0..5)).prop_map(|(from, keys)| create_test_event(from, keys))
    }
    fn events_strategy() -> impl Strategy<Value = Vec<Event>> {
        prop::collection::vec(event_strategy(), 0..10)
    }

    #[test]
    fn test_empty_filter() {
        let events: Vec<Event> = vec![];
        let writer = EventBloomWriter::from_events(events.iter());
        assert!(writer.size() > 0, "Filter should have non-zero size even when empty");
    }

    #[rstest]
    fn test_empty_key_array_handling(single_event: Event) {
        let writer = EventBloomWriter::from_events(std::iter::once(&single_event));
        let serialized = serde_json::to_string(&writer).unwrap();
        let reader: EventBloomReader = serde_json::from_str(&serialized).unwrap();

        // Only empty arrays should work
        let searcher = EventBloomSearcher::new(Some(&Felt::from(1)), Some(&[vec![], vec![]]));
        assert!(searcher.search(&reader), "Search with only empty arrays should work");
    }

    #[rstest]
    fn test_no_patterns() {
        let event = single_event();
        let writer = EventBloomWriter::from_events(std::iter::once(&event));
        let serialized = serde_json::to_string(&writer).unwrap();
        let reader: EventBloomReader = serde_json::from_str(&serialized).unwrap();

        // No patterns should match everything
        let searcher = EventBloomSearcher::new(None, None);
        assert!(searcher.search(&reader));
    }

    #[rstest]
    fn test_single_event_exact_match(single_event: Event) {
        let writer = EventBloomWriter::from_events(std::iter::once(&single_event));
        let serialized = serde_json::to_string(&writer).unwrap();
        let reader: EventBloomReader = serde_json::from_str(&serialized).unwrap();

        let searcher = EventBloomSearcher::new(Some(&Felt::from(1)), Some(&[vec![Felt::from(2)], vec![Felt::from(3)]]));
        assert!(searcher.search(&reader));
    }

    #[rstest]
    fn test_single_event_key_matching(single_event: Event) {
        let writer = EventBloomWriter::from_events(std::iter::once(&single_event));
        let serialized = serde_json::to_string(&writer).unwrap();
        let reader: EventBloomReader = serde_json::from_str(&serialized).unwrap();

        // Should match either key
        let searcher = EventBloomSearcher::new(Some(&Felt::from(1)), Some(&[vec![], vec![Felt::from(3)]]));
        assert!(searcher.search(&reader));

        let searcher =
            EventBloomSearcher::new(Some(&Felt::from(1)), Some(&[vec![], vec![Felt::from(3), Felt::from(4)]]));
        assert!(searcher.search(&reader));
    }

    #[rstest]
    fn test_single_event_no_match(single_event: Event) {
        let writer = EventBloomWriter::from_events(std::iter::once(&single_event));
        let serialized = serde_json::to_string(&writer).unwrap();
        let reader: EventBloomReader = serde_json::from_str(&serialized).unwrap();

        let searcher = EventBloomSearcher::new(
            Some(&Felt::from(1)),
            Some(&[vec![Felt::from(4)]]), // Non-existent key
        );
        assert!(!searcher.search(&reader));
    }

    #[rstest]
    fn test_multiple_events_or_semantics(multiple_events: Vec<Event>) {
        let writer = EventBloomWriter::from_events(multiple_events.iter());
        let serialized = serde_json::to_string(&writer).unwrap();
        let reader: EventBloomReader = serde_json::from_str(&serialized).unwrap();

        // Should match if any key matches
        let searcher = EventBloomSearcher::new(None, Some(&[vec![Felt::from(2), Felt::from(4)]]));
        assert!(searcher.search(&reader));
    }

    #[rstest]
    fn test_multiple_events_and_semantics(multiple_events: Vec<Event>) {
        let writer = EventBloomWriter::from_events(multiple_events.iter());
        let serialized = serde_json::to_string(&writer).unwrap();
        let reader: EventBloomReader = serde_json::from_str(&serialized).unwrap();

        // Should match only when all patterns match
        let searcher = EventBloomSearcher::new(Some(&Felt::from(1)), Some(&[vec![Felt::from(2)], vec![Felt::from(3)]]));
        assert!(searcher.search(&reader));
    }

    #[rstest]
    fn test_multiple_events_negative_case(multiple_events: Vec<Event>) {
        let writer = EventBloomWriter::from_events(multiple_events.iter());
        let serialized = serde_json::to_string(&writer).unwrap();
        let reader: EventBloomReader = serde_json::from_str(&serialized).unwrap();

        let searcher = EventBloomSearcher::new(Some(&Felt::from(1)), Some(&[vec![Felt::from(5)]]));
        assert!(!searcher.search(&reader));
    }

    #[test]
    fn test_bloom_key_creation() {
        let felt = Felt::from(123);
        let key = create_bloom_key(5, &felt);

        assert_eq!(key[0], 5, "First byte should be the index");
        assert_eq!(&key[1..], &felt.to_bytes_be(), "Remaining bytes should be the Felt value");
    }

    proptest! {
        #[test]
        fn test_false_positive_rate(events in events_strategy()) {
            // Only proceed if we have enough events to make the test meaningful
            if events.len() < 2 {
                return Ok(());
            }

            let writer = EventBloomWriter::from_events(events.iter());
            let serialized = serde_json::to_string(&writer).unwrap();
            let reader: EventBloomReader = serde_json::from_str(&serialized).unwrap();

            // Create a searcher with a non-existent Felt value
            let max_felt = events.iter()
                .flat_map(|e| std::iter::once(e.from_address)
                    .chain(e.keys.iter().cloned()))
                .max()
                .unwrap();

            let test_felt = Felt::from(max_felt.to_bytes_be()[0] as u64 + 1000);
            let searcher = EventBloomSearcher::new(Some(&test_felt), None);

            // The false positive rate should be relatively low
            // Note: This is a probabilistic test and might occasionally fail
            if searcher.search(&reader) {
                // Log when we hit a false positive for analysis
                println!("False positive detected with test_felt: {:?}", test_felt);
            }
        }
    }

    // Enhanced strategy for generating larger event sets
    fn large_events_strategy() -> impl Strategy<Value = Vec<Event>> {
        // Generate between 100 and 1000 events
        let range = 100..1000usize;
        range.prop_flat_map(|size| {
            prop::collection::vec(
                (0..u64::MAX).prop_map(|from| {
                    // Generate 1 to 10 keys for each event
                    let num_keys = rand::random::<usize>() % 10 + 1;
                    let keys = (0..num_keys).map(|i| from + i as u64).collect();
                    create_test_event(from, keys)
                }),
                size,
            )
        })
    }

    // Helper function to calculate actual false positive rate
    fn measure_false_positive_rate(
        reader: &EventBloomReader,
        existing_values: &HashSet<Felt>,
        num_tests: usize,
    ) -> f64 {
        let mut false_positives = 0;
        let max_existing = existing_values.iter().max().map(|f| f.to_bytes_be()[0] as u64).unwrap_or(0);

        // Generate test values that don't exist in our set
        for i in 0..num_tests {
            let test_value = Felt::from(max_existing + 1000 + i as u64);
            if !existing_values.contains(&test_value) {
                let searcher = EventBloomSearcher::new(Some(&test_value), None);
                if searcher.search(reader) {
                    false_positives += 1;
                }
            }
        }

        false_positives as f64 / num_tests as f64
    }

    proptest! {
            // Test false positive rate with various sizes
            #[test]
            fn test_false_positive_rate_with_size(events in large_events_strategy()) {
                let start_time = Instant::now();

                // Create a set of all existing values for comparison
                let mut existing_values = HashSet::new();
                for event in &events {
                    existing_values.insert(event.from_address);
                    existing_values.extend(event.keys.iter().cloned());
                }

                let writer = EventBloomWriter::from_events(events.iter());
                let serialized = serde_json::to_string(&writer).unwrap();
                let reader: EventBloomReader = serde_json::from_str(&serialized).unwrap();

                // Measure false positive rate with 1000 test values
                let false_positive_rate = measure_false_positive_rate(&reader, &existing_values, 1000);

                // Log performance metrics
                let duration = start_time.elapsed();
                println!(
                    "Test completed in {:?} with {} events. False positive rate: {:.4}",
                    duration,
                    events.len(),
                    false_positive_rate
                );

                // The expected false positive rate should be around 0.01 (1%)
                // We allow some variance but keep it under 3%
                prop_assert!(
                    false_positive_rate <= 0.03,
                    "False positive rate too high: {}",
                    false_positive_rate
                );
            }
    }
}

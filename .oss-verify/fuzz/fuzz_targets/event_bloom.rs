// Copyright 2026 Madara contributors. SPDX-License-Identifier: Apache-2.0
//! Event bloom filters must not miss an event key that was inserted.

#![no_main]

use libfuzzer_sys::fuzz_target;
use mp_bloom_filter::{AtomicBitStore, BloomFilter, PreCalculatedHashes};
use siphasher::sip::SipHasher;

const HASH_COUNT: u8 = 7; // Matches Madara's EventBloomWriter.

fuzz_target!(|data: &[u8]| {
    let size = u64::from(data.first().copied().unwrap_or(0)) + 1;
    let filter = BloomFilter::<SipHasher, AtomicBitStore>::new(size, HASH_COUNT);
    let mut items = Vec::new();
    let mut input = data.get(1..).unwrap_or_default();

    while !input.is_empty() && items.len() < 16 {
        let len = usize::from(input[0] % 34);
        input = &input[1..];
        if input.len() < len {
            break;
        }
        let item = &input[..len];
        filter.add(&item);
        items.push(item);
        input = &input[len..];
    }

    let filter = filter.finalize();
    for item in items {
        // An inserted key must never be absent, with either query method.
        if !filter.might_contain(&item) {
            std::process::abort();
        }
        let hashes = PreCalculatedHashes::new::<SipHasher, _>(HASH_COUNT, &item);
        if !filter.might_contain_hashes(&hashes) {
            std::process::abort();
        }
    }
});

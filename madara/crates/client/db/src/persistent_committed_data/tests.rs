use super::*;
use blockifier::execution::syscalls::committed_data::CommittedDataSet;
use std::collections::{BTreeMap, HashMap};

#[test]
fn persistent_updates_match_full_rebuilds_and_keep_every_historical_proof() -> Result<()> {
    let mut disk = HashMap::<u64, Vec<u8>>::new();
    let mut next = 1;
    let mut reference = BTreeMap::new();
    let mut roots = Vec::new();
    let mut base = None;
    // Mixed insertions, zero upserts, repeated values and branches above/below existing roots.
    for tick in 0..40u64 {
        let mut patch = BTreeMap::new();
        for i in 0..17u64 {
            let key = Felt::from((i * 97 + tick * 43) % 503);
            patch.insert(key, Felt::from(if i % 5 == 0 { 0 } else { tick + i }));
        }
        if tick == 1 {
            patch.insert(Felt::MAX, Felt::ZERO);
        }
        let expected_changes: Vec<_> = patch
            .iter()
            .filter_map(|(&key, &new)| {
                reference.get(&key).copied().filter(|old| *old != new).map(|old| (key, old, new))
            })
            .collect();
        reference.extend(patch.clone());
        let mut entries: Vec<_> = patch.into_iter().collect();
        let mut tree = Tree::new(next, 1024, |id| Ok(disk.get(&id).cloned()));
        let id = match base {
            Some(id) => tree.update(id, &mut entries)?,
            None => tree.build(&mut entries)?,
        };
        assert_eq!(
            tree.changes, expected_changes,
            "Archive deltas contain exactly the changed existing leaves in key order"
        );
        let expected = CommittedDataSet::new(reference.iter().map(|(k, v)| (*k, *v)).collect())?;
        let (root, count) = tree.root(id)?;
        assert_eq!(root, expected.root());
        assert_eq!(count as usize, reference.len());
        let written: Vec<_> = tree.nodes.iter().enumerate().map(|(i, n)| (next + i as u64, n.encode())).collect();
        next = tree.next_id();
        drop(tree);
        disk.extend(written);
        roots.push((id, root, reference.clone()));
        base = Some(id);
    }
    // Drop operation caches and read all versions from encoded records.
    for (id, root, map) in &roots {
        let mut tree = Tree::new(next, 1024, |id| Ok(disk.get(&id).cloned()));
        tree.check_root(*id, *root, map.len() as u32)?;
        for (&key, &value) in map {
            let proof = tree.witness(*id, *root, key)?.unwrap();
            assert_eq!(proof.value, value);
            assert!(proof.verify());
        }
        assert!(tree.witness(*id, *root, Felt::from(999))?.is_none());
        let expected: Vec<_> = map.iter().map(|(k, v)| (*k, *v)).collect();
        for start in (0..map.len()).step_by(11) {
            let mut page = Vec::new();
            tree.page(*id, start as u32, 11, &mut page)?;
            assert_eq!(page, expected[start..(start + 11).min(map.len())]);
        }
    }
    let (id, _, map) = roots.last().unwrap();
    let mut tree = Tree::new(next, 1024, |id| Ok(disk.get(&id).cloned()));
    let mut same: Vec<_> = map.iter().map(|(k, v)| (*k, *v)).collect();
    assert_eq!(tree.update(*id, &mut same)?, *id);
    assert!(tree.nodes.is_empty());
    assert!(tree.update(*id, &mut [(Felt::ONE, Felt::ZERO); 2]).is_err());
    Ok(())
}

#[test]
fn committed_cache_retains_current_frontier_not_obsolete_versions() -> Result<()> {
    let mut disk = HashMap::new();
    let mut values: Vec<_> = (0..128u32).map(|k| (Felt::from(k), Felt::from(k))).collect();
    let mut tree = Tree::with_cache(1, 256, |_| Ok(None), NodeCache::default());
    let mut id = tree.build(&mut values)?;
    let mut next = tree.next_id();
    for (i, n) in tree.nodes.iter().enumerate() {
        disk.insert(i as u64 + 1, n.encode());
    }
    let mut cache = tree.into_committed_cache();
    let mut prior = Vec::new();
    for tick in 0..30u32 {
        let old_id = id;
        let reads = std::cell::Cell::new(0);
        let mut tree = Tree::with_cache(
            next,
            256,
            |id| {
                reads.set(reads.get() + 1);
                Ok(disk.get(&id).cloned())
            },
            cache,
        );
        let old_root = tree.root(old_id)?.0;
        prior.push((old_id, old_root));
        values[(tick % 128) as usize].1 = Felt::from(1000 + tick);
        id = tree.update(old_id, &mut [values[(tick % 128) as usize]])?;
        assert_eq!(tree.root(id)?.0, CommittedDataSet::new(values.clone())?.root());
        assert_eq!(reads.get(), 0, "Warm updates must reuse authenticated immutable nodes");
        let records: Vec<_> = tree.nodes.iter().enumerate().map(|(i, n)| (next + i as u64, n.encode())).collect();
        next = tree.next_id();
        cache = tree.into_committed_cache();
        disk.extend(records);
        assert_eq!(cache.0.len(), 255, "Obsolete nodes must not displace the current frontier");
    }
    // Eviction is only a performance change; persisted history remains authoritative.
    let mut tiny = NodeCache::default();
    tiny.0.resize(NonZeroUsize::new(2).unwrap());
    let mut tree = Tree::with_cache(next, 256, |id| Ok(disk.get(&id).cloned()), tiny);
    for (id, root) in prior {
        assert!(tree.witness(id, root, Felt::from(5))?.unwrap().verify());
    }
    Ok(())
}

#[test]
fn corrupt_records_and_allocation_limits_fail_closed() -> Result<()> {
    let mut tree = Tree::new(1, 4, |_| Ok(None));
    let mut entries = vec![(Felt::ONE, Felt::TWO), (Felt::from(8), Felt::ZERO)];
    let id = tree.build(&mut entries)?;
    let (root, count) = tree.root(id)?;
    let mut disk: HashMap<_, _> = tree.nodes.iter().enumerate().map(|(i, n)| (i as u64 + 1, n.encode())).collect();
    let next = tree.next_id();
    drop(tree);
    // Tamper a value without changing the expected external root.
    disk.get_mut(&1).unwrap()[64] ^= 1;
    let mut tree = Tree::new(next, 4, |id| Ok(disk.get(&id).cloned()));
    assert!(tree.check_root(id, root, count).is_err());
    assert!(Node::decode(3, &[1; 54]).is_err());
    let mut tree = Tree::new(1, 1, |_| Ok(None));
    assert!(tree.build(&mut entries).is_err());
    let mut tree = Tree::new(u64::MAX, 4, |_| Ok(None));
    assert!(tree.build(&mut entries[..1]).is_err());
    Ok(())
}

#[test]
fn committed_node_cache_enforces_bytes_and_recent_blocks() {
    let mut config = crate::committed_data_retention::CommittedDataRetentionConfig {
        memory_blocks: 2,
        cache_bytes: NodeCache::ENTRY_BYTES * 4,
        ..Default::default()
    };
    let mut cache = NodeCache::default();
    cache.configure(&config, 10);
    for id in 1..=3 {
        cache.put(id, Node::new_leaf(Felt::from(id), Felt::ONE), true);
    }
    assert_eq!(cache.0.len(), 2); // half the shared budget belongs to the node cache
    assert!(!cache.0.contains(&1));
    cache.configure(&config, 11);
    assert_eq!(cache.0.len(), 2);
    cache.configure(&config, 12);
    assert!(cache.0.is_empty());
    cache.configure(&config, 5); // a reorg cannot move the cache clock backwards
    assert_eq!(cache.1, 12);
    config.cache_bytes = 0;
    cache.configure(&config, 12);
    cache.put(1, Node::new_leaf(Felt::ONE, Felt::ONE), true);
    assert!(cache.0.is_empty());
    config.cache_bytes = 10000;
    config.memory_blocks = 0;
    cache.configure(&config, 12);
    cache.put(1, Node::new_leaf(Felt::ONE, Felt::ONE), true);
    assert!(cache.0.is_empty());
}

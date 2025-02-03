use crate::*;
use proptest::prelude::*;
use rayon::prelude::*;
use std::collections::HashSet;
use std::hash::DefaultHasher;
use tests::utils::create_filter;

proptest! {
    #[test]
    fn proptest_parallel_insertions(ref input_set in prop::collection::vec(0u64..u64::MAX, 1000..5000)) {
        let nb_elem = input_set.len() as u64;
        let filter = create_filter::<DefaultHasher>(nb_elem);
        let input_set: HashSet<u64> = input_set.iter().cloned().collect();

        input_set.par_iter().for_each(|item| {
            filter.add(item);
        });

        let ro_filter = filter.finalize();

        let false_negatives: Vec<_> = input_set.par_iter()
            .filter(|&&item| !ro_filter.might_contain(&item))
            .collect();

        prop_assert!(false_negatives.is_empty(), "Found {} false negatives!", false_negatives.len());
    }
}

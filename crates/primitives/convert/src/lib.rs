mod felt;
mod to_felt;

pub use felt::{felt_to_u128, felt_to_u64};
pub use to_felt::ToFelt;

pub mod test {
    /// Asserts that the conversion between two types is consistent.
    /// Use this function only for testing purposes.
    pub fn assert_consistent_conversion<T1, T2>(a: T1)
    where
        T1: Clone + PartialEq + std::fmt::Debug + From<T2>,
        T2: Clone + PartialEq + std::fmt::Debug + From<T1>,
    {
        let b: T2 = a.clone().into();
        let c: T1 = b.clone().into();
        assert_eq!(a, c);
        let d: T2 = c.clone().into();
        assert_eq!(b, d);
    }
}

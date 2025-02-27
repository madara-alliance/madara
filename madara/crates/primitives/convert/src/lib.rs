mod felt;
pub mod hex_serde;
mod to_felt;

pub use felt::felt_to_h160;
pub use to_felt::{DisplayFeltAsHex, FeltHexDisplay, ToFelt};

pub mod test {
    /// Asserts that the conversion between two types is consistent.
    /// Use this function only for testing purposes.
    pub fn assert_consistent_conversion<T1, T2>(a: T1)
    where
        T1: Clone + PartialEq + std::fmt::Debug + TryFrom<T2>,
        T2: Clone + PartialEq + std::fmt::Debug + TryFrom<T1>,
        <T1 as TryFrom<T2>>::Error: std::fmt::Debug,
        <T2 as TryFrom<T1>>::Error: std::fmt::Debug,
    {
        let b: T2 = a.clone().try_into().unwrap();
        let c: T1 = b.clone().try_into().unwrap();
        assert_eq!(a, c);
        let d: T2 = c.clone().try_into().unwrap();
        assert_eq!(b, d);
    }
}

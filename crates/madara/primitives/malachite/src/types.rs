use starknet_types_core::felt::Felt;

#[repr(transparent)]
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Height(pub u64);

impl std::fmt::Display for Height {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Height").field("at", &self.0).finish()
    }
}

impl malachite_core_types::Height for Height {
    fn increment_by(&self, n: u64) -> Self {
        Self(self.0.saturating_add(n))
    }

    fn decrement_by(&self, n: u64) -> Option<Self> {
        self.0.checked_sub(n).map(Self)
    }

    fn as_u64(&self) -> u64 {
        self.0
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Address(Felt);

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Address").field("at", &self.0).finish()
    }
}

impl malachite_core_types::Address for Address {}

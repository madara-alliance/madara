use starknet_types_core::felt::Felt;

#[derive(Clone, Copy, Default, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Height {
    pub block_number: u64,
    pub fork_id: u64,
}

impl std::fmt::Display for Height {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.block_number.fmt(f)
    }
}

impl Height {
    pub fn new(block_number: u64, fork_id: u64) -> Self {
        Self { block_number, fork_id }
    }
}

impl malachite_core_types::Height for Height {
    const ZERO: Self = Height { block_number: 0, fork_id: 0 };
    const INITIAL: Self = Self::ZERO;

    fn increment_by(&self, n: u64) -> Self {
        Self { block_number: self.block_number.saturating_add(n), fork_id: self.fork_id }
    }

    fn decrement_by(&self, n: u64) -> Option<Self> {
        self.block_number.checked_sub(n).map(|block_number| Self { block_number, fork_id: self.fork_id })
    }

    fn as_u64(&self) -> u64 {
        self.block_number
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Address(pub libp2p::identity::PeerId);

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Address").field("at", &self.0).finish()
    }
}

impl Address {
    pub fn new(id: libp2p::identity::PeerId) -> Self {
        Self(id)
    }
}

impl malachite_core_types::Address for Address {}

// BUG: There seems to be an issue with the specs, which define the p2p validator address type as an
// on-chain address. This is likely to be bug, in which case this will be removed.
impl From<Address> for mp_proto::model::Address {
    fn from(value: Address) -> Self {
        // WARN: for the love of everything dear DO NOT MERGE THIS INTO MAIN until someone with
        // proper cryptographic knowledge has reviewed this. Libp2p can use different signing
        // schemas for its public and private keys and I am not sure all of these can fit in a Felt.
        // This should not error (I think?) but we might end up truncating an address and that would
        // be bad!
        Self(Felt::from_bytes_be_slice(value.0.to_bytes().as_slice()))
    }
}

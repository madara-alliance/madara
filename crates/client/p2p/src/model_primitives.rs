//! We override some of the codegen definitions to avoid useless allocations, in particular,
//! [`Felt252`], [`Hash`], [`Hash256`]. [`Uint128`], [`Address`] (felt251), [`EthereumAddress`]. TODO:  [`PeerId`]

use std::ops::{Deref, DerefMut};

use bytes::Buf;
use bytes::BufMut;
use bytes::Bytes;
use mp_convert::FeltExt;
use prost::DecodeError;
use starknet_core::types::Felt as SnFelt;
use starknet_core::types::Hash256 as SnHash256;

use crate::model;

trait WrappedBytesPrimitive: Default {
    fn to_bytes(&self, dest: &mut impl BufMut);
    fn bytes_len(&self) -> usize;
    fn from_bytes(b: Bytes) -> Result<Self, DecodeError>;
}

macro_rules! impl_message_for_wrappeed_bytes_primitive {
    ($type_name:ident) => {
        impl prost::Message for $type_name {
            fn encode_raw(&self, buf: &mut impl prost::bytes::BufMut) {
                prost::encoding::encode_key(1u32, prost::encoding::wire_type::WireType::LengthDelimited, buf);
                prost::encoding::encode_varint(self.bytes_len() as u64, buf);
                self.to_bytes(buf)
            }
            fn merge_field(
                &mut self,
                tag: u32,
                wire_type: prost::encoding::wire_type::WireType,
                buf: &mut impl prost::bytes::Buf,
                ctx: prost::encoding::DecodeContext,
            ) -> Result<(), DecodeError> {
                if tag == 1u32 {
                    let len = prost::encoding::decode_varint(buf)?;
                    if len > buf.remaining() as u64 {
                        return Err(DecodeError::new("Message size error"));
                    }
                    let len = len as usize;

                    *self = Self::from_bytes(buf.copy_to_bytes(len))?;
                    Ok(())
                } else {
                    prost::encoding::skip_field(wire_type, tag, buf, ctx)
                }
            }
            fn encoded_len(&self) -> usize {
                let len = self.bytes_len();
                prost::encoding::key_len(1u32) + prost::encoding::encoded_len_varint(len as u64) + len
            }
            fn clear(&mut self) {
                *self = Default::default();
            }
        }
    };
}

macro_rules! impl_wrapper_traits {
    ($type_name:ident => $inner:ident) => {
        impl From<$inner> for $type_name {
            fn from(value: $inner) -> Self {
                Self(value)
            }
        }
        impl From<$type_name> for $inner {
            fn from(value: $type_name) -> Self {
                value.0
            }
        }
        impl Deref for $type_name {
            type Target = $inner;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
        impl DerefMut for $type_name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Felt252(pub SnFelt);
impl_wrapper_traits!(Felt252 => SnFelt);

impl WrappedBytesPrimitive for Felt252 {
    fn to_bytes(&self, dest: &mut impl BufMut) {
        let bytes = self.0.to_bytes_be();
        let bytes = &bytes[32 - self.0.slice_be_len()..32];
        dest.put(bytes);
    }
    fn bytes_len(&self) -> usize {
        self.0.slice_be_len()
    }
    fn from_bytes(b: Bytes) -> Result<Self, DecodeError> {
        SnFelt::from_slice_be_checked(&b).map(Self).map_err(|_| DecodeError::new("Malformated felt"))
    }
}
impl_message_for_wrappeed_bytes_primitive!(Felt252);

/// A hash value representable as a [`Felt252`].
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Hash(pub SnFelt);
impl_wrapper_traits!(Hash => SnFelt);

impl WrappedBytesPrimitive for Hash {
    fn to_bytes(&self, dest: &mut impl BufMut) {
        let bytes = self.0.to_bytes_be();
        let bytes = &bytes[32 - self.0.slice_be_len()..32];
        dest.put(bytes);
    }
    fn bytes_len(&self) -> usize {
        self.0.slice_be_len()
    }
    fn from_bytes(b: Bytes) -> Result<Self, DecodeError> {
        SnFelt::from_slice_be_checked(&b).map(Self).map_err(|_| DecodeError::new("Malformated felt"))
    }
}
impl_message_for_wrappeed_bytes_primitive!(Hash);

/// 2**251
pub const ADDRESS_UPPER_BOUND: SnFelt =
    SnFelt::from_hex_unchecked("0x800000000000000000000000000000000000000000000000000000000000000");

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Address(pub SnFelt);
impl_wrapper_traits!(Address => SnFelt); // TODO: we need a proper felt251 primitive

impl WrappedBytesPrimitive for Address {
    fn to_bytes(&self, dest: &mut impl BufMut) {
        let bytes = self.0.to_bytes_be();
        let bytes = &bytes[32 - self.0.slice_be_len()..32];
        dest.put(bytes);
    }
    fn bytes_len(&self) -> usize {
        self.0.slice_be_len()
    }
    fn from_bytes(b: Bytes) -> Result<Self, DecodeError> {
        let felt = SnFelt::from_slice_be_checked(&b).map_err(|_| DecodeError::new("Malformated felt"))?;
        if felt < ADDRESS_UPPER_BOUND {
            Ok(Self(felt))
        } else {
            Err(DecodeError::new("Address out of range"))
        }
    }
}
impl_message_for_wrappeed_bytes_primitive!(Address);

#[derive(Clone, Default, PartialEq, Debug)]
pub struct EthereumAddress(pub [u8; 20]);
impl From<EthereumAddress> for SnFelt {
    fn from(value: EthereumAddress) -> Self {
        SnFelt::from_bytes_be_slice(&value.0)
    }
}
impl From<SnFelt> for EthereumAddress {
    fn from(value: SnFelt) -> Self {
        // TODO: replace the primitive type in mp_receipt
        Self(value.to_bytes_be()[32 - 20..].try_into().unwrap())
    }
}

impl WrappedBytesPrimitive for EthereumAddress {
    fn to_bytes(&self, dest: &mut impl BufMut) {
        dest.put(&self.0[..]);
    }
    fn bytes_len(&self) -> usize {
        self.0.len() // always 20 bytes
    }
    fn from_bytes(b: Bytes) -> Result<Self, DecodeError> {
        b[..].try_into().map(Self).map_err(|_| DecodeError::new("Expected exactly 32 bytes for Hash256 value"))
    }
}
impl_message_for_wrappeed_bytes_primitive!(EthereumAddress);

#[derive(Clone, PartialEq, Debug)]
pub struct Hash256(pub SnHash256);
impl_wrapper_traits!(Hash256 => SnHash256);

impl WrappedBytesPrimitive for Hash256 {
    fn to_bytes(&self, dest: &mut impl BufMut) {
        dest.put(&self.0.as_bytes()[..]);
    }
    fn bytes_len(&self) -> usize {
        self.0.as_bytes().len() // always 32 bytes
    }
    fn from_bytes(b: Bytes) -> Result<Self, DecodeError> {
        b[..]
            .try_into()
            .map(SnHash256::from_bytes)
            .map(Self)
            .map_err(|_| DecodeError::new("Expected exactly 32 bytes for Hash256 value"))
    }
}
impl_message_for_wrappeed_bytes_primitive!(Hash256);

impl Default for Hash256 {
    fn default() -> Self {
        Self(SnHash256::from_bytes(Default::default()))
    }
}

// We don't need to redefined Uint128 as it doesnt allocate anything.

pub use model::Uint128;
impl From<u128> for Uint128 {
    fn from(value: u128) -> Self {
        let b = value.to_le_bytes();
        let low = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        let high = u64::from_le_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]);
        Self { low, high }
    }
}
impl From<Uint128> for u128 {
    fn from(value: Uint128) -> Self {
        let l = value.low.to_le_bytes();
        let h = value.high.to_le_bytes();
        u128::from_le_bytes([
            l[0], l[1], l[2], l[3], l[4], l[5], l[6], l[7], h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7],
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn felt_252() {
        let test_one = |felt, serialized| {
            let mut dest = vec![];
            Felt252::from(felt).to_bytes(&mut dest);
            assert_eq!(&dest[..], serialized);
            let back = Felt252::from_bytes(dest.into()).unwrap();
            assert_eq!(SnFelt::from(back), felt);
        };
        test_one(SnFelt::from_hex_unchecked("0x0"), &[0u8; 0] as &[u8]);
        test_one(SnFelt::from_hex_unchecked("0x1"), &[1]);
        test_one(
            SnFelt::from_hex_unchecked("0x40000000000001100000000000012100000000000000000000000000000000"),
            &[64, 0, 0, 0, 0, 0, 1, 16, 0, 0, 0, 0, 0, 1, 33, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        test_one(
            SnFelt::from_hex_unchecked("0x800000000000011000000000000000000000000000000000000000000000000"),
            &[8, 0, 0, 0, 0, 0, 0, 17, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        assert!(Felt252::from_bytes(
            (&[8u8, 0, 0, 0, 0, 0, 0, 17, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
                as &[_])
                .into()
        )
        .is_err());
    }
}

//! We override some of the codegen definitions to avoid useless allocations, in particular,
//! [`Felt252`], [`Hash`], [`Hash256`]. TODO: [`Uint128`], [`Address`] (felt251), [`PeerId`], [`EthereumAddress`].

use std::ops::{Deref, DerefMut};

use bytes::Buf;
use mp_convert::FeltExt;
use prost::DecodeError;
use starknet_core::types::Felt as SnFelt;
use starknet_core::types::Hash256 as SnHash256;

fn felt_encode_raw(value: &SnFelt, buf: &mut impl prost::bytes::BufMut) {
    let bytes = value.to_bytes_be();
    let bytes = &bytes[32 - value.slice_be_len()..32];
    prost::encoding::encode_key(1u32, prost::encoding::wire_type::WireType::LengthDelimited, buf);
    prost::encoding::encode_varint(bytes.len() as u64, buf);
    buf.put(bytes);
}

fn felt_merge_field(
    value: &mut SnFelt,
    tag: u32,
    wire_type: prost::encoding::wire_type::WireType,
    buf: &mut impl prost::bytes::Buf,
    ctx: prost::encoding::DecodeContext,
) -> Result<(), DecodeError> {
    if tag == 1u32 {
        let len = prost::encoding::decode_varint(buf)?;
        if len > buf.remaining() as u64 || len > 32 {
            return Err(DecodeError::new("Felt size error"));
        }
        let len = len as usize;
        *value =
            SnFelt::from_slice_be_checked(&buf.copy_to_bytes(len)).map_err(|_| DecodeError::new("Malformated felt"))?;
        Ok(())
    } else {
        prost::encoding::skip_field(wire_type, tag, buf, ctx)
    }
}

fn felt_encoded_len(value: &SnFelt) -> usize {
    let len = value.slice_be_len();
    prost::encoding::key_len(1u32) + prost::encoding::encoded_len_varint(len as u64) + len
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

macro_rules! felt_newtype {
    { pub struct $type_name:ident(pub SnFelt); } => {
        #[derive(Default, Clone, PartialEq, Debug)]
        pub struct $type_name(pub SnFelt);
        impl prost::Message for $type_name {
            fn encode_raw(&self, buf: &mut impl prost::bytes::BufMut) {
                felt_encode_raw(&self.0, buf)
            }
            fn merge_field(
                &mut self,
                tag: u32,
                wire_type: prost::encoding::wire_type::WireType,
                buf: &mut impl prost::bytes::Buf,
                ctx: prost::encoding::DecodeContext,
            ) -> Result<(), DecodeError> {
                felt_merge_field(self, tag, wire_type, buf, ctx)
            }
            fn encoded_len(&self) -> usize {
                felt_encoded_len(&self.0)
            }
            fn clear(&mut self) {
                *self = Default::default();
            }
        }
        impl_wrapper_traits!($type_name => SnFelt);
    }
}

felt_newtype! {
    pub struct Felt252(pub SnFelt);
}
felt_newtype! {
    pub struct Hash(pub SnFelt);
}

// TODO: bad bad bad
felt_newtype! {
    pub struct Address(pub SnFelt);
}

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
impl prost::Message for EthereumAddress {
    fn encode_raw(&self, buf: &mut impl prost::bytes::BufMut) {
        prost::encoding::encode_key(1u32, prost::encoding::wire_type::WireType::LengthDelimited, buf);
        prost::encoding::encode_varint(self.0.len() as u64, buf);
        buf.put(&self.0[..]);
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
            self.0 = buf.copy_to_bytes(len as usize)[..]
                .try_into()
                .map_err(|_| DecodeError::new("Expected exactly 32 bytes for Hash256 value"))?;
            Ok(())
        } else {
            prost::encoding::skip_field(wire_type, tag, buf, ctx)
        }
    }
    fn encoded_len(&self) -> usize {
        let len = 20;
        prost::encoding::key_len(1u32) + prost::encoding::encoded_len_varint(len as u64) + len
    }
    fn clear(&mut self) {
        *self = Default::default();
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Hash256(pub SnHash256);
impl prost::Message for Hash256 {
    fn encode_raw(&self, buf: &mut impl prost::bytes::BufMut) {
        let bytes = self.0.as_bytes();
        prost::encoding::encode_key(1u32, prost::encoding::wire_type::WireType::LengthDelimited, buf);
        prost::encoding::encode_varint(bytes.len() as u64, buf);
        buf.put(&bytes[..]);
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
            self.0 = SnHash256::from_bytes(
                buf.copy_to_bytes(len as usize)[..]
                    .try_into()
                    .map_err(|_| DecodeError::new("Expected exactly 32 bytes for Hash256 value"))?,
            );
            Ok(())
        } else {
            prost::encoding::skip_field(wire_type, tag, buf, ctx)
        }
    }
    fn encoded_len(&self) -> usize {
        let len = 32;
        prost::encoding::key_len(1u32) + prost::encoding::encoded_len_varint(len as u64) + len
    }
    fn clear(&mut self) {
        *self = Default::default();
    }
}
impl_wrapper_traits!(Hash256 => SnHash256);

impl Default for Hash256 {
    fn default() -> Self {
        Self(SnHash256::from_bytes(Default::default()))
    }
}

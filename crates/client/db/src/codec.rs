use std::io::{self, Cursor, Read, Write};

use starknet_api::block::BlockHash;
use starknet_api::core::{ClassHash, ContractAddress, Nonce, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::TransactionHash;
use starknet_types_core::felt::Felt;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Decode error")]
    DecodeError,
    #[error("Encode error")]
    EncodeError,
}

pub trait Encode {
    fn encode(&self) -> Result<Vec<u8>, Error>;
}

pub trait Decode {
    fn decode(bytes: &[u8]) -> Result<Self, Error>
    where
        Self: Sized;
}

impl Encode for Felt {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        bincode::serialize(self).map_err(|_| Error::EncodeError)
    }
}

impl Decode for Felt {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        bincode::deserialize(bytes).map_err(|_| Error::DecodeError)
    }
}

impl Encode for StarkFelt {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Vec::new();
        self.serialize(&mut buffer).map_err(|_| Error::EncodeError)?;
        Ok(buffer)
    }
}

impl Decode for StarkFelt {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut cursor = std::io::Cursor::new(bytes);
        StarkFelt::deserialize(&mut cursor).ok_or(Error::DecodeError)
    }
}

impl Encode for PatriciaKey {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        self.key().encode()
    }
}

impl Decode for PatriciaKey {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let inner = StarkFelt::decode(bytes)?;
        inner.try_into().map_err(|_| Error::DecodeError)
    }
}

macro_rules! impl_for_wrapper {
    ($arg:ty) => {
        impl Encode for $arg {
            fn encode(&self) -> Result<Vec<u8>, Error> {
                self.0.encode()
            }
        }
        impl Decode for $arg {
            fn decode(bytes: &[u8]) -> Result<Self, Error> {
                Ok(Self(Decode::decode(bytes)?))
            }
        }
    };
}

impl_for_wrapper!(BlockHash);
impl_for_wrapper!(ClassHash);
impl_for_wrapper!(TransactionHash);
impl_for_wrapper!(ContractAddress);

impl Encode for Nonce {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        let nonce = u64::try_from(self.0).map_err(|_| Error::EncodeError)?;
        nonce.encode()
    }
}

impl Decode for Nonce {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let nonce = u64::decode(bytes)?;
        Ok(Nonce(StarkFelt::from(nonce)))
    }
}

impl Encode for u64 {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Vec::new();
        serialize_vlq(*self, &mut buffer).map_err(|_| Error::EncodeError)?;
        Ok(buffer)
    }
}

impl Decode for u64 {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut cursor = Cursor::new(bytes);
        deserialize_vlq(&mut cursor).map_err(|_| Error::DecodeError)
    }
}

impl Encode for u32 {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        u64::from(*self).encode()
    }
}

impl Decode for u32 {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        u64::decode(bytes)?.try_into().map_err(|_| Error::DecodeError)
    }
}

/// Write a variable-length quantity to the writer from an u64.
/// The value is written as a sequence of 7-bit bytes, with the most significant bits first.
/// The most significant bit of each byte is set if there are more bytes to read.
fn serialize_vlq(mut value: u64, writer: &mut impl Write) -> io::Result<()> {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        writer.write_all(&[byte])?;
        if value == 0 {
            break;
        }
    }
    Ok(())
}

/// Read a variable-length quantity from the reader into an u64.
/// The value is read as a sequence of 7-bit bytes, with the most significant bits first.
/// The most significant bit of each byte is set if there are more bytes to read.
fn deserialize_vlq(reader: &mut impl Read) -> io::Result<u64> {
    let mut value: u64 = 0;
    let mut shift = 0;
    loop {
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        value |= ((byte[0] & 0x7F) as u64) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vlq() {
        let mut buffer = Vec::new();
        serialize_vlq(0, &mut buffer).unwrap();
        assert_eq!(buffer, [0]);
        buffer.clear();

        serialize_vlq(127, &mut buffer).unwrap();
        assert_eq!(buffer, [127]);
        buffer.clear();

        serialize_vlq(128, &mut buffer).unwrap();
        assert_eq!(buffer, [0x80, 1]);
        buffer.clear();

        serialize_vlq(300, &mut buffer).unwrap();
        assert_eq!(buffer, [0xAC, 2]);
        buffer.clear();

        serialize_vlq(16384, &mut buffer).unwrap();
        assert_eq!(buffer, [0x80, 0x80, 1]);
        buffer.clear();

        serialize_vlq(2097152, &mut buffer).unwrap();
        assert_eq!(buffer, [0x80, 0x80, 0x80, 1]);
        buffer.clear();

        let mut cursor = Cursor::new(&[0x80, 0x80, 0x80, 1]);
        assert_eq!(deserialize_vlq(&mut cursor).unwrap(), 2097152);
        assert_eq!(cursor.position(), 4);
        cursor.set_position(0);

        assert_eq!(deserialize_vlq(&mut cursor).unwrap(), 2097152);
        assert_eq!(cursor.position(), 4);
    }

    #[test]
    fn test_encode_decode_starkfelt() {
        let value = StarkFelt::from(42_u64);
        let bytes = value.encode().unwrap();
        let decoded = StarkFelt::decode(&bytes).unwrap();
        assert_eq!(value, decoded);
    }

    #[test]
    fn test_encode_decode_u64() {
        let value = 42_u64;
        let bytes = value.encode().unwrap();
        let decoded = u64::decode(&bytes).unwrap();
        assert_eq!(value, decoded);
    }

    #[test]
    fn test_encode_felt() {
        let felt = Felt::MAX;
        let encoded = bincode::serialize(&felt).unwrap();
        let decoded: Felt = bincode::deserialize(&encoded).unwrap();
        assert_eq!(felt, decoded);
    }
}

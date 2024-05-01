use std::io::{self, Cursor, Read, Write};

use parity_scale_codec::{Decode as DecodeParity, Encode as EncodeParity};
use starknet_api::core::{ClassHash, CompiledClassHash, Nonce};
use starknet_api::hash::StarkFelt;

use super::history::History;
use super::primitives::contract::StorageContractData;
use super::primitives::contract_class::StorageContractClassData;

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

impl Encode for History<StarkFelt> {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Vec::new();
        for &(index, value) in &self.0 {
            serialize_vlq(index, &mut buffer).map_err(|_| Error::EncodeError)?;
            value.serialize(&mut buffer).map_err(|_| Error::EncodeError)?;
        }
        Ok(buffer)
    }
}

impl Decode for History<StarkFelt> {
    fn decode(bytes: &[u8]) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let mut cursor = Cursor::new(bytes);
        let mut history = Vec::new();
        while (cursor.position() as usize) < bytes.len() {
            let index = deserialize_vlq(&mut cursor).map_err(|_| Error::DecodeError)?;
            let value = StarkFelt::deserialize(&mut cursor).ok_or(Error::DecodeError)?;
            history.push((index, value));
        }
        Ok(History(history))
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

impl Encode for ClassHash {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        Encode::encode(&self.0)
    }
}

impl Decode for CompiledClassHash {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        Ok(CompiledClassHash(Decode::decode(bytes)?))
    }
}

impl Encode for CompiledClassHash {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        Encode::encode(&self.0)
    }
}

impl Decode for ClassHash {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        Ok(ClassHash(Decode::decode(bytes)?))
    }
}

impl Encode for Nonce {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        Encode::encode(&self.0)
    }
}

impl Decode for Nonce {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        Ok(Nonce(Decode::decode(bytes)?))
    }
}

impl Encode for StorageContractClassData {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        Ok(self::EncodeParity::encode(&self))
    }
}

impl Decode for StorageContractClassData {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        self::DecodeParity::decode(&mut &bytes[..]).map_err(|_| Error::DecodeError)
    }
}

// TODO: use encode and decode on History<StarkFelt> instead of bincode
impl Encode for StorageContractData {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        bincode::serialize(self).map_err(|_| Error::EncodeError)
    }
}

// TODO: use encode and decode on History<StarkFelt> instead of bincode
impl Decode for StorageContractData {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        bincode::deserialize(bytes).map_err(|_| Error::DecodeError)
    }
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
        let decoded: StarkFelt = StarkFelt::decode(&bytes).unwrap();
        assert_eq!(value, decoded);
    }

    #[test]
    fn test_encode_decode_history() {
        let mut history = History::default();
        history.push(0, StarkFelt::from(42_u64)).unwrap();
        history
            .push(
                42,
                StarkFelt::try_from("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7").unwrap(),
            )
            .unwrap();
        history
            .push(
                u64::MAX,
                StarkFelt::try_from("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d").unwrap(),
            )
            .unwrap();
        let bytes = history.encode().unwrap();
        let decoded = History::decode(&bytes).unwrap();
        assert_eq!(history.0, decoded.0);
    }
}

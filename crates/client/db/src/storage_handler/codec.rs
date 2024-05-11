use std::io::{self, Cursor, Read, Write};

use starknet_api::core::ContractAddress;
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;

use super::history::History;

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

impl Encode for u64 {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Vec::new();
        serialize_vlq(*self, &mut buffer).map_err(|_| Error::EncodeError)?;
        Ok(buffer)
    }
}

impl Encode for History<StarkFelt> {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Vec::new();
        buffer.write_all(&self.last_index.to_le_bytes()).map_err(|_| Error::EncodeError)?;
        for &(index, value) in &self.values {
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
        let mut values = Vec::new();
        let mut last_index_bytes = [0; 8];
        cursor.read_exact(&mut last_index_bytes).map_err(|_| Error::DecodeError)?;
        let last_index = u64::from_le_bytes(last_index_bytes);
        while (cursor.position() as usize) < bytes.len() {
            let index = deserialize_vlq(&mut cursor).map_err(|_| Error::DecodeError)?;
            let value = StarkFelt::deserialize(&mut cursor).ok_or(Error::DecodeError)?;
            values.push((index, value));
        }
        Ok(History { last_index, values })
    }
}

impl Encode for (ContractAddress, StorageKey) {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Vec::new();
        self.0.serialize(&mut buffer).map_err(|_| Error::EncodeError)?;
        self.1.serialize(&mut buffer).map_err(|_| Error::EncodeError)?;
        Ok(buffer)
    }
}

/// Add a new value to an encoded history without decoding it.
pub fn add_to_history_encoded(history_encoded: &mut Vec<u8>, index: u64, value: StarkFelt) -> Result<(), Error> {
    // If the history is empty, we can add the new last_index
    if history_encoded.is_empty() {
        history_encoded.extend(index.to_le_bytes());
    }
    // If the history is not empty, we need to check if we insert in the right order
    // and update the last_index
    else {
        let mut cursor = Cursor::new(&history_encoded);
        let mut last_index_bytes = [0; 8];
        cursor.read_exact(&mut last_index_bytes).map_err(|_| Error::DecodeError)?;
        let last_index = u64::from_le_bytes(last_index_bytes);
        if index <= last_index {
            return Err(Error::EncodeError);
        }
        let index_bytes = index.to_le_bytes();
        history_encoded.iter_mut().take(8).zip(index_bytes.iter()).for_each(|(a, b)| *a = *b);
    }

    // Add the new value with the index to the history
    history_encoded.extend(index.encode()?);
    history_encoded.extend(value.encode()?);

    Ok(())
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
    fn test_encode_decode_history() {
        let mut history = History::default();
        history.push(1, StarkFelt::from(42_u64)).unwrap();
        history
            .push(
                42,
                StarkFelt::try_from("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7").unwrap(),
            )
            .unwrap();
        history
            .push(
                43,
                StarkFelt::try_from("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d").unwrap(),
            )
            .unwrap();

        let mut bytes = history.encode().unwrap();
        add_to_history_encoded(&mut bytes, 66, StarkFelt::from(42_u64)).unwrap();
        history.push(66, StarkFelt::from(42_u64)).unwrap();
        let decoded = History::decode(&bytes).unwrap();
        assert_eq!(history, decoded);
    }

    #[test]
    fn test_add_to_history_encoded() {
        let mut history = History::new(0, StarkFelt::from(42_u64));
        history.push(90, StarkFelt::from(123456_u64)).unwrap();

        let mut bytes: Vec<u8> = vec![];
        add_to_history_encoded(&mut bytes, 0, StarkFelt::from(42_u64)).unwrap();
        add_to_history_encoded(&mut bytes, 90, StarkFelt::from(123456_u64)).unwrap();

        let decoded = History::decode(&bytes).unwrap();
        assert_eq!(history, decoded);
    }
}

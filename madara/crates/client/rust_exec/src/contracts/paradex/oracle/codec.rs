use starknet_types_core::felt::Felt;

use crate::contracts::paradex::schema::oracle_types::TickData;
use crate::contracts::ExecutionError;

pub(super) fn take_felt(input: &[Felt]) -> Result<Felt, ExecutionError> {
    if input.is_empty() {
        return Err(ExecutionError::ExecutionFailed("calldata underflow".to_string()));
    }
    Ok(input[0])
}

pub(super) fn decode_felt_array(input: &[Felt]) -> Result<(Vec<Felt>, &[Felt]), ExecutionError> {
    let len = take_felt(input)?;
    let len_u32 = felt_to_u32(len)? as usize;
    if input.len() < 1 + len_u32 {
        return Err(ExecutionError::ExecutionFailed("array underflow".to_string()));
    }
    let items = input[1..1 + len_u32].to_vec();
    Ok((items, &input[1 + len_u32..]))
}

pub(super) fn decode_tick_data_array(input: &[Felt]) -> Result<(Vec<TickData>, &[Felt]), ExecutionError> {
    let len = take_felt(input)?;
    let len_u32 = felt_to_u32(len)? as usize;
    let required = 1 + len_u32 * 4;
    if input.len() < required {
        return Err(ExecutionError::ExecutionFailed("tick data array underflow".to_string()));
    }

    let mut ticks = Vec::with_capacity(len_u32);
    for chunk in input[1..required].chunks_exact(4) {
        ticks.push(TickData {
            asset_key: chunk[0],
            asset_value: chunk[1],
            decimals: chunk[2],
            last_updated_timestamp: chunk[3],
        });
    }

    Ok((ticks, &input[required..]))
}

fn felt_to_u32(value: Felt) -> Result<u32, ExecutionError> {
    let bytes = value.to_bytes_be();
    if bytes[..28].iter().any(|b| *b != 0) {
        return Err(ExecutionError::ExecutionFailed("value too large for u32".to_string()));
    }
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&bytes[28..]);
    Ok(u32::from_be_bytes(arr))
}

pub(super) fn felt_to_i128(value: Felt) -> i128 {
    // Cairo i128 negatives are stored as PRIME - abs(value); PRIME mod 2^128 == 1.
    const MAX_I128_U128: u128 = u128::MAX >> 1;
    let max_i128_felt = Felt::from(MAX_I128_U128);
    let bytes = value.to_bytes_be();
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[16..32]);
    let mut out = i128::from_be_bytes(arr);
    if value > max_i128_felt {
        out -= 1;
    }
    out
}

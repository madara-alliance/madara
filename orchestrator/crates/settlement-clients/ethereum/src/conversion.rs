use std::fmt::Write;

use alloy::dyn_abi::parser::Error;
use alloy::eips::eip4844::BYTES_PER_BLOB;
use alloy::primitives::{FixedBytes, U256};
use c_kzg::{Blob, KzgCommitment, KzgProof, KzgSettings};
use color_eyre::eyre::ContextCompat;
use color_eyre::Result as EyreResult;

/// Converts a `&[[u8; 32]]` to `Vec<U256>`.
/// Pads with zeros if any inner slice is shorter than 32 bytes.
pub(crate) fn vec_u8_32_to_vec_u256(slices: &[[u8; 32]]) -> EyreResult<Vec<U256>> {
    slices.iter().map(|slice| slice_u8_to_u256(slice)).collect()
}

/// Converts a `&[u8]` to `U256`.
pub(crate) fn slice_u8_to_u256(slice: &[u8]) -> EyreResult<U256> {
    U256::try_from_be_slice(slice).wrap_err_with(|| "could not convert &[u8] to U256".to_string())
}

/// Function to convert a slice of u8 to a padded hex string
/// Function only takes a slice of length up to 32 elements
/// Pads the value on the right side with zeros only if the converted string has lesser than 64
/// characters.
pub(crate) fn to_padded_hex(slice: &[u8]) -> String {
    assert!(slice.len() <= 32, "Slice length must not exceed 32");
    let hex = slice.iter().fold(String::new(), |mut output, byte| {
        // 0: pads with zeros
        // 2: specifies the minimum width (2 characters)
        // x: formats the number as lowercase hexadecimal
        // writes a byte value as a two-digit hexadecimal number (padded with a leading zero if necessary)
        // to the specified output.
        let _ = write!(output, "{byte:02x}");
        output
    });
    format!("{:0<64}", hex)
}

/// To get the input data
///
/// Function to construct the transaction's `input data` for updating the state in the core
/// contract. HEX Concatenation: MethodId, Offset, length for program_output, lines count,
/// program_output, length for kzg_proof, kzg_proof All 64 chars, if lesser padded from left with 0s
pub fn get_input_data_for_eip_4844(program_output: Vec<[u8; 32]>, kzg_proof: [u8; 48]) -> Result<String, Error> {
    // bytes4(keccak256(bytes("updateStateKzgDA(uint256[],bytes[])")))
    let method_id_hex = "0x507ee528";

    // offset for updateStateKzgDA is 64
    let offset: u64 = 64;
    let offset_hex = format!("{:0>64x}", offset);

    // program_output
    let program_output_length = program_output.len();
    let program_output_hex = u8_32_slice_to_hex_string(&program_output);

    // length for program_output: 3*64 [offset, length, lines all have 64 char length] + length of
    // program_output
    let length_program_output = (3 * 64 + program_output_hex.len()) / 2;
    let length_program_output_hex = format!("{:0>64x}", length_program_output);

    // lines count for program_output
    let lines_count_hex = format!("{:0>64x}", program_output_length);

    // length of KZG proof
    let length_kzg_hex = format!("{:0>64x}", kzg_proof.len());

    // length of total kzg inputs in the vec
    // hardcoded as of now
    // TODO : need to update this when we are integrating the 0.13.2 updated spec with AR (Applicative
    // recursion)
    let length_kzg_output = format!("{:0>64x}", 1);
    // Offset for 1st KZG proof starts at 32 in our case
    let kzg_proof_offset = format!("{:0>64x}", 32);
    // KZG proof
    let kzg_proof_hex = u8_48_to_hex_string(kzg_proof);

    let input_data = method_id_hex.to_string()
        + &offset_hex
        + &length_program_output_hex
        + &lines_count_hex
        + &program_output_hex
        + &length_kzg_output
        + &kzg_proof_offset
        + &length_kzg_hex
        + &kzg_proof_hex;

    Ok(input_data)
}

pub(crate) fn u8_32_slice_to_hex_string(data: &[[u8; 32]]) -> String {
    data.iter().fold(String::new(), |mut output, arr| {
        // Convert the array to a hex string
        let hex = arr.iter().fold(String::new(), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        });

        // Ensure the hex string is exactly 64 characters (32 bytes)
        let _ = write!(output, "{hex:0>64}");
        output
    })
}

pub(crate) fn u8_48_to_hex_string(data: [u8; 48]) -> String {
    // Split the array into two parts
    let (first_32, last_16) = data.split_at(32);

    // Convert and pad each part
    let first_hex = to_padded_hex(first_32);
    let second_hex = to_padded_hex(last_16);

    // Concatenate the two hex strings
    first_hex + &second_hex
}

/// To prepare the sidecar for EIP 4844 transaction
pub(crate) async fn prepare_sidecar(
    state_diff: &[Vec<u8>],
    trusted_setup: &KzgSettings,
) -> EyreResult<(Vec<FixedBytes<BYTES_PER_BLOB>>, Vec<FixedBytes<48>>, Vec<FixedBytes<48>>)> {
    let mut sidecar_blobs = vec![];
    let mut sidecar_commitments = vec![];
    let mut sidecar_proofs = vec![];

    for blob_data in state_diff {
        let fixed_size_blob: [u8; BYTES_PER_BLOB] = blob_data.as_slice().try_into()?;

        let blob = Blob::new(fixed_size_blob);

        let commitment = KzgCommitment::blob_to_kzg_commitment(&blob, trusted_setup)?;

        let proof = KzgProof::compute_blob_kzg_proof(&blob, &commitment.to_bytes(), trusted_setup)?;

        sidecar_blobs.push(FixedBytes::new(fixed_size_blob));
        sidecar_commitments.push(FixedBytes::new(commitment.to_bytes().into_inner()));
        sidecar_proofs.push(FixedBytes::new(proof.to_bytes().into_inner()));
    }

    Ok((sidecar_blobs, sidecar_commitments, sidecar_proofs))
}

#[cfg(test)]
mod tests {

    use std::fs;
    use std::path::Path;

    use color_eyre::eyre::eyre;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::typical(&[
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF
    ], U256::from_str_radix("00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF", 16).unwrap())]
    #[case::minimum(&[0; 32], U256::ZERO)]
    #[case::maximum(&[0xFF; 32], U256::MAX)]
    #[case::short(&[0xFF; 16], U256::from_be_slice(&[0xFF; 16]))]
    #[case::empty(&[], U256::ZERO)]
    #[should_panic(expected = "could not convert &[u8] to U256")]
    #[case::over(&[0xFF; 33],U256::from_be_slice(&[0xFF;32]))]
    fn slice_u8_to_u256_all_working_and_failing_cases(#[case] slice: &[u8], #[case] expected: U256) {
        assert_eq!(slice_u8_to_u256(slice).expect("slice_u8_to_u256 failed"), expected)
    }

    #[rstest]
    #[case::empty(&[], vec![])]
    #[case::single(
        &[[1; 32]],
        vec![U256::from_be_slice(&[1; 32])]
    )]
    #[case::multiple(
        &[
            [1; 32],
            [2; 32],
            [3; 32],
        ],
        vec![
            U256::from_be_slice(&[1; 32]),
            U256::from_be_slice(&[2; 32]),
            U256::from_be_slice(&[3; 32]),
        ]
    )]
    #[case::mixed(
        &[
            [0xFF; 32],
            [0x00; 32],
            [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ],
        vec![
            U256::MAX,
            U256::ZERO,
            U256::from_be_slice(&[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        ]
    )]
    fn vec_u8_32_to_vec_u256_works(#[case] slices: &[[u8; 32]], #[case] expected: Vec<U256>) {
        match vec_u8_32_to_vec_u256(slices) {
            Ok(response) => {
                assert_eq!(response, expected);
            }
            Err(e) => {
                panic!("{}", e);
            }
        }
    }

    #[rstest]
    #[case::empty(&[], "0".repeat(64))]
    #[case::typical(&[0xFF,0xFF,0xFF,0xFF], format!("{}{}", "ff".repeat(4), "0".repeat(56)))]
    #[case::big(&[0xFF; 32], format!("{}", "ff".repeat(32)))]
    #[should_panic(expected = "Slice length must not exceed 32")]
    #[case::exceeding(&[0xFF; 40], format!("{}", "ff".repeat(32)))]
    fn to_hex_string_working_and_failing_cases(#[case] slice: &[u8], #[case] expected: String) {
        let result = to_padded_hex(slice);
        assert_eq!(result, expected);
        assert!(expected.len() == 64);
    }

    #[rstest]
    #[case::typical([
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
        31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
    ],
    "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f3000000000000000000000000000000000")]
    #[case::single_value(
        [0xFF;48],
        format!("{}{}","ff".repeat(48), "00".repeat(16))
    )]
    fn u8_48_to_hex_string_works(#[case] slice: [u8; 48], #[case] expected: String) {
        let result = u8_48_to_hex_string(slice);
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::typical(
        vec![
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32],
            [32, 31, 30, 29, 28, 27, 26, 25, 24, 23, 22, 21, 20, 19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1],
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 0, 0]
        ],
        "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20201f1e1d1c1b1a191817161514131211100f0e0d0c0b0a0908070605040302010102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e0000"
    )]
    #[case::single_value(
        vec![
            [0xFF;32],
            [0xF5;32],
        ],
        format!("{}{}", "ff".repeat(32), "f5".repeat(32))
    )]
    fn u8_32_slice_to_hex_string_works(#[case] slice: Vec<[u8; 32]>, #[case] expected: String) {
        let result = u8_32_slice_to_hex_string(&slice);
        assert_eq!(result, expected);
    }

    // block_no here are Ethereum(mainnet) blocks, we are creating sidecar and validating
    // the function by matching pre-existing commitments against computed.
    // https://etherscan.io/tx/0x4e012b119391bdc192653bfee9758c432ea6f35ff23f8af60a7dca4664383dfc
    // https://etherscan.io/tx/0x96470b890833c5ae51622bd6efca98d8eec3b4a66402c34be3cdcacf006eb9a0
    #[rstest]
    #[case("20462788")]
    #[case("20462818")]
    #[tokio::test]
    async fn prepare_sidecar_works(#[case] fork_block_no: String) {
        // Trusted Setup
        let current_path = std::env::current_dir().unwrap().to_str().unwrap().to_string();

        let trusted_setup_file_path = current_path.clone() + "/src/trusted_setup.txt";
        let trusted_setup = KzgSettings::load_trusted_setup_file(Path::new(trusted_setup_file_path.as_str()))
            .expect("issue while loading the trusted setup");

        // Blob Data
        let blob_data_file_path =
            format!("{}{}{}{}", current_path.clone(), "/src/test_data/blob_data/", fork_block_no, ".txt");
        let blob_data = fs::read_to_string(blob_data_file_path).expect("Failed to read the blob data txt file");

        // Blob Commitment
        let blob_commitment_file_path =
            format!("{}{}{}{}", current_path.clone(), "/src/test_data/blob_commitment/", fork_block_no, ".txt");
        let blob_commitment =
            fs::read_to_string(blob_commitment_file_path).expect("Failed to read the blob data txt file");

        // Blob Proof
        let blob_proof_file_path =
            format!("{}{}{}{}", current_path.clone(), "/src/test_data/blob_proof/", fork_block_no, ".txt");
        let blob_proof = fs::read_to_string(blob_proof_file_path).expect("Failed to read the blob data txt file");

        fn hex_string_to_u8_vec(hex_str: &str) -> color_eyre::Result<Vec<u8>> {
            // Remove any spaces or non-hex characters from the input string
            let cleaned_str: String = hex_str.chars().filter(|c| c.is_ascii_hexdigit()).collect();

            // Convert the cleaned hex string to a Vec<u8>
            let mut result = Vec::new();
            for chunk in cleaned_str.as_bytes().chunks(2) {
                if let Ok(byte_val) = u8::from_str_radix(std::str::from_utf8(chunk)?, 16) {
                    result.push(byte_val);
                } else {
                    return Err(eyre!("Error parsing hex string: {}", cleaned_str));
                }
            }

            Ok(result)
        }

        let blob_data_vec = vec![hex_string_to_u8_vec(&blob_data).unwrap()];

        match prepare_sidecar(&blob_data_vec, &trusted_setup).await {
            Ok(result) => {
                let (_, sidecar_commitments, sidecar_proofs) = result;
                // Assumption: since only 1 blob, thus only 1 commitment and proof
                assert_eq!(blob_commitment, sidecar_commitments[0].to_string());
                assert_eq!(blob_proof, sidecar_proofs[0].to_string());
            }
            Err(err) => {
                panic!("{}", err.to_string())
            }
        }
    }
}

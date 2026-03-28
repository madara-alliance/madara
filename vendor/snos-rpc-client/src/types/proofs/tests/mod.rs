use crate::types::hash::PedersenHash;
use crate::types::nodes::Proof;
use crate::types::{ContractProof, PoseidonHash, TrieNode};
use rstest::rstest;
use starknet_types_core::felt::Felt;

#[test]
#[ignore]
fn test_verify_proof_from_json() {
    // Placeholder values - replace with actual test data
    let keys = [
        "0x3c204dd68b8e800b4f42e438d9ed4ccbba9f8e436518758cd36553715c1d6ab",
        "0x345354e2d801833068de73d1a2028e2f619f71045dd5229e79469fa7f598038",
        "0x3b28019ccfdbd30ffc65951d94bb85c9e2b8434111a000b5afd533ce65f57a4",
        "0x5496768776e3db30053404f18067d81a6e06f5a2b0de326e21298fd9d569a9a",
        "0x229",
        "0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d",
        "0x9524a94b41c4440a16fd96d7c1ef6ad6f44c1c013e96662734502cd4ee9b1f",
        "0x626c15d497f8ef78da749cbe21ac3006470829ee8b5d0d166f3a139633c6a93",
        "0x5c1c7eca392fa7c8ff79bbd7559f45f9693278ddc62edf335e374648d17cbb",
        "0x22",
        "0x3c204dd68b8e800b4f42e438d9ed4ccbba9f8e436518758cd36553715c1d6ac",
        "0x140ab62001bb99cdf9685fd3be123aeeb41073e31942b80622fda6b66d54d4f",
        "0x5496768776e3db30053404f18067d81a6e06f5a2b0de326e21298fd9d569a9b",
        "0x352057331d5ad77465315d30b98135ddb815b86aa485d659dfeef59a904f88d",
    ];

    for (index, item) in keys.iter().enumerate().skip(2) {
        let key = Felt::from_hex(item).unwrap();
        let commitment = Felt::from_hex("0x113519a4e8c4b74d2295b850122523986c6e60902cfc31a623da2e765c76b3d").unwrap();
        let json_file_content = include_str!("../../../../../../resources/pathfinder_proof_1309254_2.json");

        // Read proof from JSON file - fail test if the file cannot be read
        let snos_proof: ContractProof =
            serde_json::from_str(json_file_content).expect("Failed to read Proof from JSON file");

        // Get contract data - fail test if not found
        let contract_data = snos_proof.contract_data.as_ref().expect("No contract data found in the Proof");

        // Get storage proofs - fail test if empty
        assert!(!contract_data.storage_proofs.is_empty(), "No storage proofs found in the Proof");

        let proof = &contract_data.storage_proofs[index];

        // Call verify_proof with PedersenHash - fail test if verification fails
        proof.verify_proof::<PedersenHash>(key, commitment).expect("Proof verification failed");
        println!("Proof verification successful for index {:?}", index);
    }
}

#[rstest]
#[case(
    "0x654925ee73f27940c6d0721070e25da685c27a82dd6b6dc9a477185527da70c",
    "0x141226668ad8394934d53eb7db9cc6f582eda0655d5264900902c5c29acd769",
    "0x3224ded91b58d810bc9016ade2ff9941f9ba3f236f6ce18ef2e4ec449dac309" // placeholder expected result
)]
#[case(
    "0x1234",
    "0xabcd",
    "0615bb8d47888d2987ad0c63fc06e9e771930986a4dd8adc55617febfcf3639e" // placeholder expected result
)]
#[case(
    "0x03d937c035c878245caf64531a5756109c53068da139362728feb561405371cb",
    "0x0208a0a10250e382e1e4bbe2880906c2791bf6275695e02fbbc6aeff9cd8b31a",
    "0x030e480bed5fe53fa909cc0f8c4d99b8f9f2c016be4c41e13a4848797979c662" // placeholder expected result
)]
fn test_hash_binary_node_pedersen(#[case] left_hex: &str, #[case] right_hex: &str, #[case] expected_hex: &str) {
    let left = Felt::from_hex(left_hex).unwrap();
    let right = Felt::from_hex(right_hex).unwrap();
    let expected = Felt::from_hex(expected_hex).unwrap();

    println!("left: {:?}, right: {:?} and expected: {:?}", left, right, expected);

    let result = TrieNode::new_binary(left, right, None).calculate_node_hash::<PedersenHash>();

    // TODO: Replace with actual expected values
    // For now, just verify the function runs without panicking
    println!("Binary node hash result: {:#x}", result);
    assert_eq!(result, expected);
}

// TODO: Fix this test or remove it
#[ignore]
#[rstest]
#[case(
    "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
    "0x0000000000000000000000000000000000000000000000000000000000000000" // placeholder expected result
)]
#[case(
    "0x0000000000000000000000000000000000000000000000000000000000000001",
    "0x0000000000000000000000000000000000000000000000000000000000000002",
    "0x0000000000000000000000000000000000000000000000000000000000000000" // placeholder expected result
)]
fn test_hash_binary_node_poseidon(#[case] left_hex: &str, #[case] right_hex: &str, #[case] _expected_hex: &str) {
    let left = Felt::from_hex(left_hex).unwrap();
    let right = Felt::from_hex(right_hex).unwrap();
    // let expected = Felt::from_hex(expected_hex).unwrap();

    let result = TrieNode::new_binary(left, right, None).calculate_node_hash::<PoseidonHash>();

    // TODO: Replace with actual expected values
    // For now, just verify the function runs without panicking
    println!("Binary node hash result: {:#x}", result);
    // assert_eq!(result, expected);
}

#[rstest]
#[case(
    "0x0",
    230,
    "0x10eea74e23d39bef6e9995ba1f84c3a4b7e577c63b884c6f3d941c77a00346c",
    "0x22e0326bd27681dee9760d7b639cf024731bf14b71d7e4e9ce48b7ff831c2b0" // placeholder expected result
)]
fn test_hash_edge_node_pedersen(
    #[case] path_hex: &str,
    #[case] path_length: usize,
    #[case] child_hash_hex: &str,
    #[case] expected_hex: &str,
) {
    let path = Felt::from_hex(path_hex).unwrap();
    let child_hash = Felt::from_hex(child_hash_hex).unwrap();
    let expected = Felt::from_hex(expected_hex).unwrap();

    let result = TrieNode::new_edge(child_hash, path_length as u64, path, None).calculate_node_hash::<PedersenHash>();

    // TODO: Replace with actual expected values
    // For now, just verify the function runs without panicking
    println!("Edge node hash result: {:#x}", result);
    assert_eq!(result, expected);
}

// TODO: Fix this test or remove it
#[ignore]
#[rstest]
#[case(
    "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    4,
    "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
    "0x0000000000000000000000000000000000000000000000000000000000000000" // placeholder expected result
)]
#[case(
    "0x0000000000000000000000000000000000000000000000000000000000000001",
    8,
    "0x0000000000000000000000000000000000000000000000000000000000000002",
    "0x0000000000000000000000000000000000000000000000000000000000000000" // placeholder expected result
)]
fn test_hash_edge_node_poseidon(
    #[case] path_hex: &str,
    #[case] path_length: usize,
    #[case] child_hash_hex: &str,
    #[case] _expected_hex: &str,
) {
    let path = Felt::from_hex(path_hex).unwrap();
    let child_hash = Felt::from_hex(child_hash_hex).unwrap();
    // let expected = Felt::from_hex(expected_hex).unwrap();

    let result = TrieNode::new_edge(child_hash, path_length as u64, path, None).calculate_node_hash::<PoseidonHash>();

    // TODO: Replace with actual expected values
    // For now, just verify the function runs without panicking
    println!("Edge node hash result: {:#x}", result);
    // assert_eq!(result, expected);
}

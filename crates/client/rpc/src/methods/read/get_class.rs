use cairo_vm::types::program::Program;
use dc_db::storage_handler::primitives::contract_class::{ContractClassWrapper, StorageContractClassData};
use dc_db::storage_handler::StorageView;
use dp_convert::to_stark_felt::ToStarkFelt;
use flate2::read::GzDecoder;
use jsonrpsee::core::RpcResult;
use starknet_api::core::ClassHash;
use starknet_core::types::{BlockId, CompressedLegacyContractClass, ContractClass, Felt};
use std::io::Read;

use crate::errors::StarknetRpcApiError;
use crate::utils::ResultExt;
use crate::Starknet;

pub fn get_class(starknet: &Starknet, block_id: BlockId, class_hash: Felt) -> RpcResult<ContractClass> {
    let class_hash = ClassHash(class_hash.to_stark_felt());

    // Check if the given block exists
    starknet.get_block(block_id)?;

    let class = starknet
        .backend
        .contract_class_data()
        .get(&class_hash)
        .or_internal_server_error("Failed to retrieve contract class")?
        .ok_or(StarknetRpcApiError::ClassHashNotFound)?;

    let StorageContractClassData {
        contract_class,
        abi,
        sierra_program_length,
        abi_length,
        block_number: declared_at_block,
    } = class;

    if declared_at_block >= starknet.get_block_n(block_id)? {
        return Err(StarknetRpcApiError::ClassHashNotFound.into());
    }

    let contract_class_core: ContractClass =
        ContractClassWrapper { contract_class, abi, sierra_program_length, abi_length }
            .try_into()
            .or_else_internal_server_error(|| {
                format!("Failed to convert contract class from hash '{class_hash}' to RPC contract class")
            })?;

    let contract_class = match contract_class_core {
        ContractClass::Sierra(class) => ContractClass::Sierra(class),
        ContractClass::Legacy(class) => {
            let program = from_compressed_legacy_contract_class(&class).or_else_internal_server_error(|| {
                format!("Failed to convert compressed legacy contract class to blockifier class")
            })?;

            // Log the blockifier program
            log::info!("Blockifier Program: {:?}", program);

            ContractClass::Legacy(class)
        }
    };

    Ok(contract_class)
}

// Helper function to convert compressed legacy contract class to blockifier class
pub fn from_compressed_legacy_contract_class(
    compressed_class: &CompressedLegacyContractClass,
) -> anyhow::Result<Program> {
    // Gzip decompress the program
    let mut d = GzDecoder::new(&compressed_class.program[..]);
    let mut decompressed_program = Vec::new();
    d.read_to_end(&mut decompressed_program).expect("Decompressing program failed");

    // Deserialize the program
    let program = Program::from_bytes(&decompressed_program, None).expect("Deserializing program failed");

    Ok(program)
}

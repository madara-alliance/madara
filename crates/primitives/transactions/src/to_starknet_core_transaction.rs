use std::vec::Vec;
use blockifier::transaction::objects::FeeTypeIter;
use starknet_api::transaction::{Calldata, DeclareTransaction, DeclareTransactionV0V1, DeclareTransactionV2, DeclareTransactionV3, DeployAccountTransaction, DeployAccountTransactionV1, DeployAccountTransactionV3, DeployTransaction, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV1, InvokeTransactionV3, L1HandlerTransaction, Transaction};

///Here we transform mp-transactions input into starknet-api trasnactions 

use mp_felt::Felt252Wrapper;
use starknet_core::types::DeclareTransactionV0;
use starknet_crypto::FieldElement;

fn cast_vec_of_felt_252_wrappers(data: Vec<Felt252Wrapper>) -> Vec<FieldElement> {
    // Non-copy but less dangerous than transmute
    // https://doc.rust-lang.org/std/mem/fn.transmute.html#alternatives

    // Unsafe code but all invariants are checked:

    // 1. ptr must have been allocated using the global allocator -> data is allocated with the Global
    //    allocator.
    // 2. T needs to have the same alignment as what ptr was allocated with -> Felt252Wrapper uses
    //    transparent representation of the inner type.
    // 3. The allocated size in bytes needs to be the same as the pointer -> As FieldElement and
    //    Felt252Wrapper have the same size, and capacity is taken directly from the data Vector, we
    //    will have the same allocated byte size.
    // 4. Length needs to be less than or equal to capacity -> data.len() is always less than or equal
    //    to data.capacity()
    // 5. The first length values must be properly initialized values of type T -> ok since we use data
    //    which was correctly allocated
    // 6. capacity needs to be the capacity that the pointer was allocated with -> data.as_mut_ptr()
    //    returns a pointer to memory having at least capacity initialized memory
    // 7. The allocated size in bytes must be no larger than isize::MAX -> data.capacity() will never be
    //    bigger than isize::MAX (https://doc.rust-lang.org/std/vec/struct.Vec.html#panics-7)
    let mut data = core::mem::ManuallyDrop::new(data);
    unsafe { alloc::vec::Vec::from_raw_parts(data.as_mut_ptr() as *mut FieldElement, data.len(), data.capacity()) }
}

pub fn to_starknet_core_tx(
    tx: Transaction,
    transaction_hash: FieldElement,
) -> starknet_core::types::Transaction {
    match tx {
        Transaction::Declare(tx) => {
            let tx = match tx {
                DeclareTransaction::V0(DeclareTransactionV0V1 {
                    max_fee,
                    signature,
                    nonce: _,
                    class_hash,
                    sender_address,
                }) => starknet_core::types::DeclareTransaction::V0(starknet_core::types::DeclareTransactionV0 {
                    transaction_hash,
                    max_fee: Felt252Wrapper::from(max_fee.0).into(),
                    signature: signature.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                    class_hash: Felt252Wrapper::from(class_hash.0).into(),
                    sender_address: Felt252Wrapper::from(sender_address.0).into(),
                }),
                DeclareTransaction::V1(DeclareTransactionV0V1 {
                    max_fee,
                    signature,
                    nonce,
                    class_hash,
                    sender_address,
                    ..
                }) => starknet_core::types::DeclareTransaction::V1(starknet_core::types::DeclareTransactionV1 {
                    transaction_hash,
                    max_fee: Felt252Wrapper::from(max_fee.0).into(),
                    signature: signature.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                    nonce: Felt252Wrapper::from(nonce.0).into(),
                    class_hash: Felt252Wrapper::from(class_hash.0).into(),
                    sender_address: Felt252Wrapper::from(sender_address.0).into(),
                }),
                DeclareTransaction::V2(DeclareTransactionV2 {
                    max_fee,
                    signature,
                    nonce,
                    class_hash,
                    sender_address,
                    compiled_class_hash,
                    ..
                }) => starknet_core::types::DeclareTransaction::V2(starknet_core::types::DeclareTransactionV2 {
                    transaction_hash,
                    max_fee: Felt252Wrapper::from(max_fee.0).into(),
                    signature: signature.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                    nonce: Felt252Wrapper::from(nonce.0).into(),
                    class_hash: Felt252Wrapper::from(class_hash.0).into(),
                    sender_address: Felt252Wrapper::from(sender_address.0).into(),
                    compiled_class_hash: Felt252Wrapper::from(compiled_class_hash.0).into(),
                }),
                DeclareTransaction::V3(DeclareTransactionV3 {
                    resource_bounds,
                    tip,
                    signature,
                    nonce,
                    class_hash,
                    compiled_class_hash,
                    sender_address,
                    nonce_data_availability_mode,
                    fee_data_availability_mode,
                    paymaster_data,
                    account_deployment_data,
                }) => starknet_core::types::DeclareTransaction::V3(starknet_core::types::DeclareTransactionV3 {
                    transaction_hash,
                    resource_bounds: resource_bounds.into(),
                    tip: Felt252Wrapper::from(tip.0).into().to_u64(),
                    signature: signature.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                    nonce: Felt252Wrapper::from(nonce.0).into(),
                    class_hash: Felt252Wrapper::from(class_hash.0).into(),
                    compiled_class_hash: Felt252Wrapper::from(compiled_class_hash.0).into(),
                    sender_address: Felt252Wrapper::from(sender_address.0).into(),
                    nonce_data_availability_mode: nonce_data_availability_mode.into(),
                    fee_data_availability_mode: fee_data_availability_mode.into(),
                    paymaster_data: paymaster_data.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                    account_deployment_data: account_deployment_data.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                }),

            };

            starknet_core::types::Transaction::Declare(tx)
        }
        Transaction::DeployAccount(tx) => {
            let tx = match tx {
                DeployAccountTransaction::V1(DeployAccountTransactionV1 {
                    max_fee,
                    signature,
                    nonce,
                    contract_address_salt,
                    constructor_calldata,
                    class_hash,
                    ..
                }) => starknet_core::types::DeployAccountTransaction::V1(
                    starknet_core::types::DeployAccountTransactionV1 {
                        transaction_hash,
                        max_fee: Felt252Wrapper::from(max_fee.0).into(),
                        signature: signature.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                        nonce: Felt252Wrapper::from(nonce.0).into(),
                        contract_address_salt: Felt252Wrapper::from(contract_address_salt.0).into(),
                        constructor_calldata: constructor_calldata.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                        class_hash: Felt252Wrapper::from(class_hash.0).into(),
                    },
                ),
                DeployAccountTransaction::V3(DeployAccountTransactionV3{
                    resource_bounds,
                    tip,
                    signature,
                    nonce,
                    class_hash,
                    contract_address_salt,
                    constructor_calldata,
                    nonce_data_availability_mode,
                    fee_data_availability_mode,
                    paymaster_data,
                }) => starknet_core::types::DeployAccountTransaction::V3(
                    starknet_core::types::DeployAccountTransactionV3 {
                        transaction_hash,
                        resource_bounds: resource_bounds.into(),
                        tip: Felt252Wrapper::from(tip.0).into().to_u64(),
                        signature: signature.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                        nonce: Felt252Wrapper::from(nonce.0).into(),
                        class_hash: Felt252Wrapper::from(class_hash.0).into(),
                        contract_address_salt: Felt252Wrapper::from(contract_address_salt.0).into(),
                        constructor_calldata: constructor_calldata.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                        nonce_data_availability_mode: nonce_data_availability_mode.into(),
                        fee_data_availability_mode: fee_data_availability_mode.into(),
                        paymaster_data: paymaster_data.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                    },
                ),
            };

            starknet_core::types::Transaction::DeployAccount(tx)
        }
        Transaction::Deploy(tx) => {
            let tx = starknet_core::types::DeployTransaction {
                transaction_hash,
                contract_address_salt: Felt252Wrapper::from(tx.contract_address_salt.0).into(),
                constructor_calldata: constructor_calldata.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                class_hash: Felt252Wrapper::from(tx.class_hash.0).into(),
                version: Felt252Wrapper::ZERO.into(),
            };

            starknet_core::types::Transaction::Deploy(tx)
        }
        Transaction::Invoke(tx) => {
            let tx = match tx {
                InvokeTransaction::V0(InvokeTransactionV0 {
                    max_fee,
                    signature,
                    contract_address,
                    entry_point_selector,
                    calldata,
                }) => starknet_core::types::InvokeTransaction::V0(starknet_core::types::InvokeTransactionV0 {
                    transaction_hash,
                    max_fee: Felt252Wrapper::from(max_fee.0).into(),
                    signature: signature.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                    contract_address: Felt252Wrapper::from(contract_address.0).into(),
                    entry_point_selector: Felt252Wrapper::from(entry_point_selector.0).into(),
                    calldata: calldata.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                }),
                InvokeTransaction::V1(InvokeTransactionV1{
                    max_fee,
                    signature,
                    nonce,
                    sender_address,
                    calldata,
                    ..
                }) => starknet_core::types::InvokeTransaction::V1(starknet_core::types::InvokeTransactionV1 {
                    transaction_hash,
                    max_fee: Felt252Wrapper::from(max_fee.0).into(),
                    signature: signature.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                    nonce: Felt252Wrapper::from(nonce.0).into(),
                    sender_address: Felt252Wrapper::from(sender_address.0).into(),
                    calldata: calldata.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                }),
                InvokeTransaction::V3(InvokeTransactionV3 {
                    resource_bounds,
                    tip,
                    signature,
                    nonce,
                    sender_address,
                    calldata,
                    nonce_data_availability_mode,
                    fee_data_availability_mode,
                    paymaster_data,
                    account_deployment_data,

                }) => starknet_core::types::InvokeTransaction::V3(starknet_core::types::InvokeTransactionV3 {
                    transaction_hash,
                    resource_bounds: resource_bounds.into(),
                    tip: Felt252Wrapper::from(tip.0).into().to_u64(),
                    signature: signature.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                    nonce: Felt252Wrapper::from(nonce.0).into(),
                    sender_address: Felt252Wrapper::from(sender_address.0).into(),
                    calldata: calldata.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                    nonce_data_availability_mode: nonce_data_availability_mode.into(),
                    fee_data_availability_mode: fee_data_availability_mode.into(),
                    paymaster_data: paymaster_data.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                    account_deployment_data: account_deployment_data.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
                }),
            };

            starknet_core::types::Transaction::Invoke(tx)
        }
        Transaction::L1Handler(tx) => {
            let tx = starknet_core::types::L1HandlerTransaction {
                transaction_hash,
                version: FieldElement::ZERO,
                nonce: Felt252Wrapper::from(tx.nonce).into().to_u64(),
                contract_address: Felt252Wrapper::from(tx.contract_address).into(),
                entry_point_selector: Felt252Wrapper::from(tx.entry_point_selector).into(),
                calldata: calldata.0.iter().map(|x| Felt252Wrapper::from(*x)).collect().into(),
            };

            starknet_core::types::Transaction::L1Handler(tx)
        }
    }
}

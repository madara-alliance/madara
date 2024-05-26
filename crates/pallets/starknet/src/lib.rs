//! A Substrate pallet implementation for Starknet, a decentralized, permissionless, and scalable
//! zk-rollup for general-purpose smart contracts.
//! See the [Starknet documentation](https://docs.starknet.io/) for more information.
//! The code consists of the following sections:
//! 1. Config: The trait Config is defined, which is used to configure the pallet by specifying the
//! parameters and types on which it depends. The trait also includes associated types for
//! RuntimeEvent, StateRoot, SystemHash, and TimestampProvider.
//!
//! 2. Hooks: The Hooks trait is implemented for the pallet, which includes methods to be executed
//! during the block lifecycle: on_finalize, on_initialize, on_runtime_upgrade, and offchain_worker.
//!
//! 3. Storage: Several storage items are defined, including Pending, CurrentBlock, BlockHash,
//! ContractClassHashes, ContractClasses, Nonces, StorageView, LastKnownEthBlock, and
//! FeeTokenAddress. These storage items are used to store and manage data related to the Starknet
//! pallet.
//!
//! 4. Genesis Configuration: The GenesisConfig struct is defined, which is used to set up the
//! initial state of the pallet during genesis. The struct includes fields for contracts,
//! contract_classes, storage, fee_token_address, and _phantom. A GenesisBuild implementation is
//! provided to build the initial state during genesis.
//!
//! 5. Events: A set of events are defined in the Event enum, including KeepStarknetStrange,
//! StarknetEvent, and FeeTokenAddressChanged. These events are emitted during the execution of
//! various pallet functions.
//!
//! 6.Errors: A set of custom errors are defined in the Error enum, which is used to represent
//! various error conditions during the execution of the pallet.
//!
//! 7. Dispatchable Functions: The Pallet struct implements several dispatchable functions (ping,
//! invoke, ...), which allow users to interact with the pallet and invoke state changes. These
//! functions are annotated with weight and return a DispatchResult.
// Ensure we're `no_std` when compiling for Wasm.
#![allow(clippy::large_enum_variant)]

/// An adapter for the blockifier state related traits
/// Starknet pallet.
/// Definition of the pallet's runtime storage items, events, errors, and dispatchable
/// functions.
/// Learn more about FRAME and the core library of Substrate FRAME pallets:
/// <https://docs.substrate.io/reference/frame-pallets/>
pub use pallet::*;

#[cfg(feature = "std")]
pub mod genesis_loader;

/// The Starknet pallet's runtime custom types.
pub mod types;

#[macro_use]
pub extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use frame_support::pallet_prelude::*;
use frame_system::pallet_prelude::*;
use mc_db::storage_handler;
use mc_db::storage_handler::StorageViewMut;
use mp_block::DeoxysBlock;
use mp_digest_log::DEOXYS_ENGINE_ID;
use mp_felt::{trim_hash, Felt252Wrapper};
use sp_runtime::DigestItem;
use starknet_api::core::{CompiledClassHash, ContractAddress};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;

use crate::types::{CasmClassHash, SierraClassHash};

pub(crate) const LOG_TARGET: &str = "runtime::starknet";

pub const ETHEREUM_EXECUTION_RPC: &[u8] = b"starknet::ETHEREUM_EXECUTION_RPC";
pub const ETHEREUM_CONSENSUS_RPC: &[u8] = b"starknet::ETHEREUM_CONSENSUS_RPC";

pub const SN_OS_CONFIG_HASH_VERSION: &str = "StarknetOsConfig1";

// syntactic sugar for logging.
#[macro_export]
macro_rules! log {
	($level:tt, $pattern:expr $(, $values:expr)* $(,)?) => {
		log::$level!(
			target: $crate::LOG_TARGET,
			concat!("[{:?}] 🐺 ", $pattern), <frame_system::Pallet<T>>::block_number() $(, $values)*
		)
	};
}

#[frame_support::pallet]
pub mod pallet {
    use mc_db::DeoxysBackend;

    use super::*;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Configure the pallet by specifying the parameters and types on which it depends.
    /// We're coupling the starknet pallet to the tx payment pallet to be able to override the fee
    /// mechanism and comply with starknet which uses an ER20 as fee token
    #[pallet::config]
    pub trait Config: frame_system::Config {}

    /// The Starknet pallet hooks.
    /// HOOKS
    /// # TODO
    /// * Implement the hooks.
    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// The block is being finalized.
        fn on_finalize(_n: BlockNumberFor<T>) {
            // Create a new Starknet block and store it.
            <Pallet<T>>::store_block();
        }

        /// The block is being initialized. Implement to have something happen.
        fn on_initialize(_: BlockNumberFor<T>) -> Weight {
            Weight::zero()
        }

        /// Perform a module upgrade.
        fn on_runtime_upgrade() -> Weight {
            Weight::zero()
        }
    }

    /// Starknet genesis configuration.
    #[pallet::genesis_config]
    pub struct GenesisConfig<T: Config> {
        /// The contracts to be deployed at genesis.
        /// This is a vector of tuples, where the first element is the contract address and the
        /// second element is the contract class hash.
        /// This can be used to start the chain with a set of pre-deployed contracts, for example in
        /// a test environment or in the case of a migration of an existing chain state.
        pub contracts: Vec<(ContractAddress, SierraClassHash)>,
        pub sierra_to_casm_class_hash: Vec<(SierraClassHash, CasmClassHash)>,
        pub storage: Vec<(ContractAddress, Vec<(StorageKey, StarkFelt)>)>,
        /// The address of the fee token.
        /// Must be set to the address of the fee token ERC20 contract.
        pub strk_fee_token_address: ContractAddress,
        pub eth_fee_token_address: ContractAddress,
        pub _phantom: PhantomData<T>,
    }

    /// `Default` impl required by `pallet::GenesisBuild`.
    impl<T: Config> Default for GenesisConfig<T> {
        fn default() -> Self {
            Self {
                contracts: vec![],
                sierra_to_casm_class_hash: vec![],
                storage: vec![],
                strk_fee_token_address: Default::default(),
                eth_fee_token_address: Default::default(),
                _phantom: PhantomData,
            }
        }
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            if DeoxysBackend::mapping().starknet_block_hash_from_block_number(1).unwrap().is_none() {
                let handler_contract_data = storage_handler::contract_data_mut();
                self.contracts.iter().for_each(|(contract_address, class_hash)| {
                    handler_contract_data.insert_class_hash(*contract_address, *class_hash).unwrap();
                });
                handler_contract_data.commit(0).unwrap();

                let handler_contract_class_hashes = storage_handler::contract_class_hashes_mut();
                self.sierra_to_casm_class_hash.iter().for_each(|(class_hash, compiled_class_hash)| {
                    handler_contract_class_hashes
                        .insert(*class_hash, CompiledClassHash(compiled_class_hash.0))
                        .unwrap();
                });
                handler_contract_class_hashes.commit(0).unwrap();
            }
        }
    }

    /// The Starknet pallet custom errors.
    /// ERRORS
    #[pallet::error]
    pub enum Error<T> {
        AccountNotDeployed,
        TransactionExecutionFailed,
        ClassHashAlreadyDeclared,
        ContractClassHashUnknown,
        ContractClassAlreadyAssociated,
        ContractClassMustBeSpecified,
        AccountAlreadyDeployed,
        ContractAddressAlreadyAssociated,
        InvalidContractClass,
        TooManyEmittedStarknetEvents,
        StateReaderError,
        EmitEventError,
        StateDiffError,
        ContractNotFound,
        TransactionConversionError,
        SequencerAddressNotValid,
        InvalidContractClassForThisDeclareVersion,
        Unimplemented,
        MissingRevertReason,
        MissingCallInfo,
        FailedToCreateATransactionalStorageExecution,
        L1MessageAlreadyExecuted,
        MissingL1GasUsage,
    }

    /// The Starknet pallet external functions.
    /// Dispatchable functions allows users to interact with the pallet and invoke state changes.
    /// These functions materialize as "extrinsics", which are often compared to transactions.
    /// Dispatchable functions must be annotated with a weight and must return a DispatchResult.
    #[pallet::call]
    impl<T: Config> Pallet<T> {}
}

/// The Starknet pallet internal functions.
impl<T: Config> Pallet<T> {
    /// Store a Starknet block in the blockchain.
    ///
    /// # Arguments
    ///
    /// * `block_number` - The block number.
    fn store_block() {
        let block: DeoxysBlock;
        match &frame_system::Pallet::<T>::digest().logs()[0] {
            DigestItem::PreRuntime(mp_digest_log::DEOXYS_ENGINE_ID, encoded_data) => {
                block = match DeoxysBlock::decode(&mut encoded_data.as_slice()) {
                    Ok(b) => b,
                    Err(e) => {
                        log!(error, "Failed to decode block: {:?}", e);
                        return;
                    }
                };

                let actual_block_number = block.header().block_number;
                let block_hash = Felt252Wrapper::try_from(block.header().extra_data.unwrap()).unwrap();
                let state_root = Felt252Wrapper::try_from(block.header().global_state_root).unwrap();

                let digest = DigestItem::Consensus(DEOXYS_ENGINE_ID, mp_digest_log::Log::Block(block).encode());
                frame_system::Pallet::<T>::deposit_log(digest);
                log::info!(
                    "✨ Imported #{} ({}) and updated state root ({})",
                    actual_block_number,
                    trim_hash(&block_hash),
                    trim_hash(&state_root)
                );
            }
            _ => {
                log!(info, "Block not found in store_block")
            }
        }
    }
}

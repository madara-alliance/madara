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

use alloc::str::from_utf8_unchecked;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use blockifier::blockifier::block::{BlockInfo, GasPrices};
use blockifier::context::{BlockContext, ChainInfo, FeeTokenAddresses};
use blockifier::execution::call_info::CallInfo;
use blockifier::versioned_constants::VersionedConstants;
use frame_support::pallet_prelude::*;
use frame_support::traits::Time;
use frame_system::pallet_prelude::*;
use mc_db::storage_handler;
use mc_db::storage_handler::StorageViewMut;
use mp_block::DeoxysBlock;
use mp_digest_log::DEOXYS_ENGINE_ID;
use mp_felt::{trim_hash, Felt252Wrapper};
use mp_hashers::HasherT;
use mp_sequencer_address::{InherentError, InherentType, DEFAULT_SEQUENCER_ADDRESS, INHERENT_IDENTIFIER};
use mp_storage::{StarknetStorageSchemaVersion, PALLET_STARKNET_SCHEMA};
use sp_runtime::traits::UniqueSaturatedInto;
use sp_runtime::DigestItem;
use starknet_api::block::{BlockNumber, BlockTimestamp};
use starknet_api::core::{ChainId, CompiledClassHash, ContractAddress};
use starknet_api::hash::{StarkFelt, StarkHash};
use starknet_api::state::StorageKey;
use starknet_api::transaction::{Event as StarknetEvent, MessageToL1, TransactionHash};
use starknet_crypto::FieldElement;

use crate::alloc::string::ToString;
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
    use super::*;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Configure the pallet by specifying the parameters and types on which it depends.
    /// We're coupling the starknet pallet to the tx payment pallet to be able to override the fee
    /// mechanism and comply with starknet which uses an ER20 as fee token
    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// Because this pallet emits events, it depends on the runtime's definition of an event.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// The hashing function to use.
        type SystemHash: HasherT;
        /// The block time
        type TimestampProvider: Time;
        /// The gas price
        #[pallet::constant]
        type L1GasPrices: Get<GasPrices>;
        /// A configuration for base priority of unsigned transactions.
        ///
        /// This is exposed so that it can be tuned for particular runtime, when
        /// multiple pallets send unsigned transactions.
        #[pallet::constant]
        type UnsignedPriority: Get<TransactionPriority>;
        /// A configuration for longevity of transactions.
        ///
        /// This is exposed so that it can be tuned for particular runtime to
        /// set how long transactions are kept in the mempool.
        #[pallet::constant]
        type TransactionLongevity: Get<TransactionLongevity>;
        /// A bool to disable transaction fees and make all transactions free
        #[pallet::constant]
        type DisableTransactionFee: Get<bool>;
        /// A bool to disable Nonce validation
        type DisableNonceValidation: Get<bool>;
        #[pallet::constant]
        type InvokeTxMaxNSteps: Get<u32>;
        #[pallet::constant]
        type ValidateMaxNSteps: Get<u32>;
        #[pallet::constant]
        type ProtocolVersion: Get<Felt252Wrapper>;
        #[pallet::constant]
        type ChainId: Get<Felt252Wrapper>;
        #[pallet::constant]
        type MaxRecursionDepth: Get<u32>;
        #[pallet::constant]
        type ProgramHash: Get<Felt252Wrapper>;
    }

    /// The Starknet pallet hooks.
    /// HOOKS
    /// # TODO
    /// * Implement the hooks.
    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// The block is being finalized.
        fn on_finalize(_n: BlockNumberFor<T>) {
            assert!(SeqAddrUpdate::<T>::take(), "Sequencer address must be set for the block");

            let block_number =
                UniqueSaturatedInto::<u64>::unique_saturated_into(frame_system::Pallet::<T>::block_number());

            // Create a new Starknet block and store it.
            <Pallet<T>>::store_block(block_number);
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

    // TODO: @charpa is this used in any RPC calls?
    #[pallet::storage]
    #[pallet::unbounded]
    #[pallet::getter(fn tx_events)]
    pub(super) type TxEvents<T: Config> = StorageMap<_, Identity, TransactionHash, Vec<StarknetEvent>, ValueQuery>;

    #[pallet::storage]
    #[pallet::unbounded]
    #[pallet::getter(fn tx_messages)]
    pub(super) type TxMessages<T: Config> = StorageMap<_, Identity, TransactionHash, Vec<MessageToL1>, ValueQuery>;

    #[pallet::storage]
    #[pallet::unbounded]
    #[pallet::getter(fn tx_revert_error)]
    pub(super) type TxRevertError<T: Config> = StorageMap<_, Identity, TransactionHash, String, OptionQuery>;

    /// The last processed Ethereum block number for L1 messages consumption.
    /// This is used to avoid re-processing the same Ethereum block multiple times.
    /// This is used by the offchain worker.
    /// # TODO
    /// * Find a more relevant name for this.
    // TODO: do we actually need this?
    #[pallet::storage]
    #[pallet::unbounded]
    #[pallet::getter(fn last_known_eth_block)]
    pub(super) type LastKnownEthBlock<T: Config> = StorageValue<_, u64>;

    /// The address of the fee token ERC20 contract.
    #[pallet::storage]
    #[pallet::unbounded]
    #[pallet::getter(fn fee_token_addresses)]
    pub(super) type FeeTokens<T: Config> = StorageValue<_, FeeTokenAddresses, ValueQuery>;

    /// Current sequencer address.
    #[pallet::storage]
    #[pallet::unbounded]
    #[pallet::getter(fn sequencer_address)]
    pub type SequencerAddress<T: Config> = StorageValue<_, ContractAddress, ValueQuery>;

    /// Ensure the sequencer address was updated for this block.
    #[pallet::storage]
    #[pallet::unbounded]
    #[pallet::getter(fn seq_addr_update)]
    pub type SeqAddrUpdate<T: Config> = StorageValue<_, bool, ValueQuery>;

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
            frame_support::storage::unhashed::put::<StarknetStorageSchemaVersion>(
                PALLET_STARKNET_SCHEMA,
                &StarknetStorageSchemaVersion::V1,
            );

            if !storage_handler::block_hash().get(0).is_ok() {
                println!("Genesis build: block hash");
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

            LastKnownEthBlock::<T>::set(None);
            // Set the fee token address from the genesis config.
            FeeTokens::<T>::set(FeeTokenAddresses {
                strk_fee_token_address: self.strk_fee_token_address,
                eth_fee_token_address: self.eth_fee_token_address,
            });
            SeqAddrUpdate::<T>::put(true);
        }
    }

    /// The Starknet pallet events.
    /// EVENTS
    /// See: `<https://docs.substrate.io/main-docs/build/events-errors/>`
    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        KeepStarknetStrange,
        /// Regular Starknet event
        StarknetEvent(StarknetEvent),
        /// Emitted when fee token address is changed.
        /// This is emitted by the `set_fee_token_address` extrinsic.
        /// [old_fee_token_address, new_fee_token_address]
        FeeTokenAddressChanged {
            old_fee_token_address: ContractAddress,
            new_fee_token_address: ContractAddress,
        },
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
    impl<T: Config> Pallet<T> {
        /// Set the current block author's sequencer address.
        ///
        /// This call should be invoked exactly once per block. It will set a default value at
        /// the finalization phase, if this call hasn't been invoked by that time.
        ///
        /// The dispatch origin for this call must be `Inherent`.
        #[pallet::call_index(0)]
        #[pallet::weight((0, DispatchClass::Mandatory))]
        pub fn set_sequencer_address(origin: OriginFor<T>, addr: [u8; 32]) -> DispatchResult {
            ensure_none(origin)?;
            // The `SeqAddrUpdate` storage item is initialized to `true` in the genesis build. In
            // block 1 we skip the storage update check, and the `on_finalize` hook
            // updates the storage item to `false`. Initializing the storage item with
            // `false` causes the `on_finalize` hook to panic.
            if UniqueSaturatedInto::<u64>::unique_saturated_into(frame_system::Pallet::<T>::block_number()) > 1 {
                assert!(!SeqAddrUpdate::<T>::exists(), "Sequencer address can be updated only once in the block");
            }

            let addr = StarkFelt::new(addr).map_err(|_| Error::<T>::SequencerAddressNotValid)?;
            let addr = ContractAddress(addr.try_into().map_err(|_| Error::<T>::SequencerAddressNotValid)?);
            SequencerAddress::<T>::put(addr);
            SeqAddrUpdate::<T>::put(true);
            Ok(())
        }
    }

    #[pallet::inherent]
    impl<T: Config> ProvideInherent for Pallet<T> {
        type Call = Call<T>;
        type Error = InherentError;
        const INHERENT_IDENTIFIER: InherentIdentifier = INHERENT_IDENTIFIER;

        fn create_inherent(data: &InherentData) -> Option<Self::Call> {
            let inherent_data = data
                .get_data::<InherentType>(&INHERENT_IDENTIFIER)
                .expect("Sequencer address inherent data not correctly encoded")
                .unwrap_or(DEFAULT_SEQUENCER_ADDRESS);
            Some(Call::set_sequencer_address { addr: inherent_data })
        }

        fn is_inherent(call: &Self::Call) -> bool {
            matches!(call, Call::set_sequencer_address { .. })
        }
    }
}

/// The Starknet pallet internal functions.
impl<T: Config> Pallet<T> {
    /// Creates a [BlockContext] object. The [BlockContext] is needed by the blockifier to execute
    /// properly the transaction. Substrate caches data so it's fine to call multiple times this
    /// function, only the first transaction/block will be "slow" to load these data.
    pub fn get_block_context() -> BlockContext {
        let block_number = UniqueSaturatedInto::<u64>::unique_saturated_into(frame_system::Pallet::<T>::block_number());
        let block_timestamp = Self::block_timestamp();

        let fee_token_addresses = Self::fee_token_addresses();
        let sequencer_address = Self::sequencer_address();

        let chain_id = ChainId(Self::chain_id_str());
        let gas_prices = T::L1GasPrices::get();

        BlockContext::new_unchecked(
            &BlockInfo {
                block_number: BlockNumber(block_number),
                block_timestamp: BlockTimestamp(block_timestamp),
                sequencer_address,
                gas_prices,
                // TODO
                // I have no idea what this is, let's say we did not use any for now
                use_kzg_da: false,
            },
            &ChainInfo { chain_id, fee_token_addresses },
            // TODO
            // I'm clueless on what those values should be
            VersionedConstants::latest_constants(),
        )
    }

    /// convert chain_id
    #[inline(always)]
    pub fn chain_id_str() -> String {
        unsafe { from_utf8_unchecked(&T::ChainId::get().0.to_bytes_be()).to_string() }
    }

    /// Get the block hash of the previous block.
    ///
    /// # Arguments
    ///
    /// * `current_block_number` - The number of the current block.
    ///
    /// # Returns
    ///
    /// The block hash of the parent (previous) block or 0 if the current block is 0.
    #[inline(always)]
    pub fn parent_block_hash(current_block_number: u64) -> Felt252Wrapper {
        match storage_handler::block_hash().get(current_block_number) {
            Ok(Some(block_hash)) => block_hash,
            _ => Felt252Wrapper::ZERO,
        }
    }

    /// Get the current block timestamp in seconds.
    ///
    /// # Returns
    ///
    /// The current block timestamp in seconds.
    #[inline(always)]
    pub fn block_timestamp() -> u64 {
        let timestamp_in_millisecond: u64 = T::TimestampProvider::now().unique_saturated_into();
        timestamp_in_millisecond / 1000
    }

    #[inline(always)]
    pub fn event_count() -> u128 {
        TxEvents::<T>::iter_values().map(|v| v.len() as u128).sum()
    }

    /// Returns a storage keys and values of a given contract
    pub fn get_storage_from(contract_address: ContractAddress) -> Result<Vec<(StorageKey, StarkFelt)>, DispatchError> {
        Ok(storage_handler::contract_storage_trie()
            .get_storage(&contract_address)
            .map_err(|_| Error::<T>::ContractNotFound)?)
    }

    /// Store a Starknet block in the blockchain.
    ///
    /// # Arguments
    ///
    /// * `block_number` - The block number.
    fn store_block(block_number: u64) {
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

                let block_hash = Felt252Wrapper::try_from(block.header().extra_data.unwrap()).unwrap();
                let state_root = Felt252Wrapper::try_from(block.header().global_state_root).unwrap();

                let mut handler_block_number = storage_handler::block_number();
                let mut handler_block_hash = storage_handler::block_hash();

                handler_block_number.insert(&block_hash, block_number).unwrap();
                handler_block_hash.insert(block_number, &block_hash).unwrap();

                let digest = DigestItem::Consensus(DEOXYS_ENGINE_ID, mp_digest_log::Log::Block(block).encode());
                frame_system::Pallet::<T>::deposit_log(digest);
                log::info!(
                    "✨ Imported #{} ({}) and updated state root ({})",
                    block_number,
                    trim_hash(&block_hash),
                    trim_hash(&state_root)
                );
            }
            _ => {
                log!(info, "Block not found in store_block")
            }
        }
    }

    /// Aggregate L2 > L1 messages from the call info.
    ///
    /// # Arguments
    ///
    /// * `tx_hash` - The hash of the transaction being processed
    /// * `call_info` — A ref to the call info structure.
    /// * `next_order` — Next expected message order, has to be 0 for a top level invocation
    ///
    /// # Returns
    ///
    /// Next expected message order
    fn aggregate_messages_in_call_info(tx_hash: TransactionHash, call_info: &CallInfo, next_order: usize) -> usize {
        let mut message_idx = 0;
        let mut inner_call_idx = 0;
        let mut next_order = next_order;

        loop {
            // Store current call's messages as long as they have sequential orders
            if message_idx < call_info.execution.l2_to_l1_messages.len() {
                let ordered_message = &call_info.execution.l2_to_l1_messages[message_idx];
                if ordered_message.order == next_order {
                    let message = MessageToL1 {
                        from_address: call_info.call.storage_address,
                        to_address: ordered_message.message.to_address,
                        payload: ordered_message.message.payload.clone(),
                    };
                    TxMessages::<T>::append(tx_hash, message);
                    next_order += 1;
                    message_idx += 1;
                    continue;
                }
            }

            // Go deeper to find the continuation of the sequence
            if inner_call_idx < call_info.inner_calls.len() {
                next_order =
                    Self::aggregate_messages_in_call_info(tx_hash, &call_info.inner_calls[inner_call_idx], next_order);
                inner_call_idx += 1;
                continue;
            }

            // At this point we have iterated over all sequential messages and visited all internal calls
            break;
        }

        next_order
    }

    /// Emit events from the call info.
    ///
    /// # Arguments
    ///
    /// * `call_info` — A ref to the call info structure.
    /// * `next_order` — Next expected event order, has to be 0 for a top level invocation
    ///
    /// # Returns
    ///
    /// Next expected event order
    #[inline(always)]
    fn emit_events_in_call_info(tx_hash: TransactionHash, call_info: &CallInfo, next_order: usize) -> usize {
        let mut event_idx = 0;
        let mut inner_call_idx = 0;
        let mut next_order = next_order;

        loop {
            // Emit current call's events as long as they have sequential orders
            if event_idx < call_info.execution.events.len() {
                let ordered_event = &call_info.execution.events[event_idx];
                if ordered_event.order == next_order {
                    let event = StarknetEvent {
                        from_address: call_info.call.storage_address,
                        content: ordered_event.event.clone(),
                    };
                    Self::deposit_event(Event::<T>::StarknetEvent(event.clone()));
                    TxEvents::<T>::append(tx_hash, event);
                    next_order += 1;
                    event_idx += 1;
                    continue;
                }
            }

            // Go deeper to find the continuation of the sequence
            if inner_call_idx < call_info.inner_calls.len() {
                next_order =
                    Self::emit_events_in_call_info(tx_hash, &call_info.inner_calls[inner_call_idx], next_order);
                inner_call_idx += 1;
                continue;
            }

            // At this point we have iterated over all sequential events and visited all internal calls
            break;
        }

        next_order
    }

    pub fn emit_and_store_tx_and_fees_events(
        tx_hash: TransactionHash,
        execute_call_info: &Option<CallInfo>,
        fee_transfer_call_info: &Option<CallInfo>,
    ) {
        if let Some(call_info) = execute_call_info {
            Self::emit_events_in_call_info(tx_hash, call_info, 0);
            Self::aggregate_messages_in_call_info(tx_hash, call_info, 0);
        }
        if let Some(call_info) = fee_transfer_call_info {
            Self::emit_events_in_call_info(tx_hash, call_info, 0);
            Self::aggregate_messages_in_call_info(tx_hash, call_info, 0);
        }
    }

    pub fn chain_id() -> Felt252Wrapper {
        T::ChainId::get()
    }

    pub fn program_hash() -> Felt252Wrapper {
        T::ProgramHash::get()
    }

    pub fn config_hash() -> StarkHash {
        Felt252Wrapper::from(T::SystemHash::compute_hash_on_elements(&[
            FieldElement::from_byte_slice_be(SN_OS_CONFIG_HASH_VERSION.as_bytes()).unwrap(),
            T::ChainId::get().into(),
            Felt252Wrapper::from(Self::fee_token_addresses().eth_fee_token_address.0.0).0,
        ]))
        .into()
    }

    pub fn is_transaction_fee_disabled() -> bool {
        T::DisableTransactionFee::get()
    }
}

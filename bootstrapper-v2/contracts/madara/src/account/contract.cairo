// SPDX-License-Identifier: MIT
// OpenZeppelin Contracts for Cairo v2.0.0 (presets/src/account.cairo)

/// # Account Preset
///
/// OpenZeppelin's upgradeable account which can change its public key and declare, deploy, or call
/// contracts. Supports outside execution by implementing SRC9.
///
/// This contract is exactly taken from the OpenZeppelin preset in openzeppelin_account::presets::AccountUpgradeable.
/// We have made the following modifications:
/// - Added a deploy_contract function for contract deployment functionality
/// - Removed the upgrade component as it was not needed for our use case
#[starknet::contract(account)]
pub mod AccountUpgradeable {
    use core::hash::{HashStateExTrait, HashStateTrait};
    use core::poseidon::PoseidonTrait;
    use openzeppelin_account::AccountComponent;
    use openzeppelin_account::extensions::SRC9Component;
    use openzeppelin_introspection::src5::SRC5Component;
    use starknet::syscalls::deploy_syscall;
    use starknet::{ClassHash, ContractAddress, SyscallResultTrait, get_caller_address};

    component!(path: AccountComponent, storage: account, event: AccountEvent);
    component!(path: SRC5Component, storage: src5, event: SRC5Event);
    component!(path: SRC9Component, storage: src9, event: SRC9Event);

    // Account Mixin
    #[abi(embed_v0)]
    pub(crate) impl AccountMixinImpl =
        AccountComponent::AccountMixinImpl<ContractState>;
    impl AccountInternalImpl = AccountComponent::InternalImpl<ContractState>;

    // SRC9
    #[abi(embed_v0)]
    impl OutsideExecutionV2Impl =
        SRC9Component::OutsideExecutionV2Impl<ContractState>;
    impl OutsideExecutionInternalImpl = SRC9Component::InternalImpl<ContractState>;


    #[storage]
    pub struct Storage {
        #[substorage(v0)]
        pub account: AccountComponent::Storage,
        #[substorage(v0)]
        pub src5: SRC5Component::Storage,
        #[substorage(v0)]
        pub src9: SRC9Component::Storage,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        #[flat]
        AccountEvent: AccountComponent::Event,
        #[flat]
        SRC5Event: SRC5Component::Event,
        #[flat]
        SRC9Event: SRC9Component::Event,
        ContractDeployed: ContractDeployed,
    }

    #[derive(Drop, PartialEq, starknet::Event)]
    struct ContractDeployed {
        address: ContractAddress,
        deployer: ContractAddress,
        from_zero: bool,
        class_hash: ClassHash,
        calldata: Span<felt252>,
        salt: felt252,
    }

    #[constructor]
    pub fn constructor(ref self: ContractState, public_key: felt252) {
        self.account.initializer(public_key);
        self.src9.initializer();
    }


    #[external(v0)]
    fn deploy_contract(
        ref self: ContractState,
        class_hash: ClassHash,
        salt: felt252,
        from_zero: bool,
        calldata: Span<felt252>,
    ) -> ContractAddress {
        let deployer: ContractAddress = get_caller_address();
        let mut _salt: felt252 = salt;
        if !from_zero {
            let mut hash_state = PoseidonTrait::new();
            _salt = hash_state.update_with(deployer).update_with(salt).finalize();
        }

        let (address, _) = deploy_syscall(class_hash, _salt, calldata, from_zero).unwrap_syscall();

        self.emit(ContractDeployed { address, deployer, from_zero, class_hash, calldata, salt });
        return address;
    }
}

// SPDX-License-Identifier: MIT
// OpenZeppelin Contracts for Cairo v0.15.1 (presets/account.cairo)

#[starknet::contract(account)]
pub mod TestContract {
    use openzeppelin_account::AccountComponent;
    use openzeppelin_introspection::src5::SRC5Component;
    use openzeppelin_upgrades::UpgradeableComponent;
    use openzeppelin_upgrades::interface::IUpgradeable;
    use starknet::ClassHash;

    component!(path: AccountComponent, storage: account, event: AccountEvent);
    component!(path: SRC5Component, storage: src5, event: SRC5Event);
    component!(path: UpgradeableComponent, storage: upgradeable, event: UpgradeableEvent);

    #[abi(embed_v0)]
    pub(crate) impl AccountMixinImpl =
        AccountComponent::AccountMixinImpl<ContractState>;
    impl AccountInternalImpl = AccountComponent::InternalImpl<ContractState>;
    impl UpgradeableInternalImpl = UpgradeableComponent::InternalImpl<ContractState>;

    #[storage]
    struct Storage {
        #[substorage(v0)]
        account: AccountComponent::Storage,
        #[substorage(v0)]
        src5: SRC5Component::Storage,
        #[substorage(v0)]
        upgradeable: UpgradeableComponent::Storage
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        #[flat]
        AccountEvent: AccountComponent::Event,
        #[flat]
        SRC5Event: SRC5Component::Event,
        #[flat]
        UpgradeableEvent: UpgradeableComponent::Event,
        CalledFromL1: CalledFromL1,
    }

    #[constructor]
    pub fn constructor(ref self: ContractState, public_key: felt252) {
        self.account.initializer(public_key);
    }

    pub fn temp() {
        let _x = 6;
    }

    #[abi(embed_v0)]
    impl UpgradeableImpl of IUpgradeable<ContractState> {
        fn upgrade(ref self: ContractState, new_class_hash: ClassHash) {
            self.account.assert_only_self();
            self.upgradeable.upgrade(new_class_hash);
        }
    }

    #[derive(Drop, starknet::Event)]
    pub struct CalledFromL1 {
        pub from_l1_address: felt252,
        pub arg1: felt252,
        pub arg2: felt252,
    }

    pub fn l1_handler_entrypoint(ref self: ContractState, from_l1_address: felt252, arg1: felt252, arg2: felt252) {
        let sender = starknet::get_caller_address();
        assert(sender.is_zero(), 'Must be called from L1 (get_caller_address = 0)');

        self.emit(CalledFromL1 {
            from_l1_address,
            arg1,
            arg2,
        });
    }
}

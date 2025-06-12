/// Simple contract for managing balance.
#[starknet::contract]
mod MadaraFactory {
    use core::array::ArrayTrait;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::syscalls::deploy_syscall;
    use starknet::{ClassHash, ContractAddress, EthAddress, SyscallResultTrait};
    use crate::interfaces::replaceable::{
        EICData, IReplaceableDispatcher, IReplaceableDispatcherTrait, ImplementationData,
    };
    use crate::interfaces::roles::{IRolesDispatcher, IRolesDispatcherTrait};
    use crate::madara_factory::interface::IMadaraFactory;

    #[storage]
    struct Storage {
        token_bridge_class_hash: ClassHash,
        eic_class_hash: ClassHash,
        erc20_class_hash: ClassHash,
        l1_eth_bridge_address: EthAddress,
        l1_erc20_bridge_address: EthAddress,
        owner: ContractAddress,
    }

    #[constructor]
    fn constructor(
        ref self: ContractState,
        token_bridge_class: ClassHash,
        eic_class_hash: ClassHash,
        erc20_class_hash: ClassHash,
        l1_eth_bridge_address: EthAddress,
        l1_erc20_bridge_address: EthAddress,
        initial_owner: ContractAddress,
    ) {
        self.token_bridge_class_hash.write(token_bridge_class);
        self.eic_class_hash.write(eic_class_hash);
        self.erc20_class_hash.write(erc20_class_hash);
        self.l1_eth_bridge_address.write(l1_eth_bridge_address);
        self.l1_erc20_bridge_address.write(l1_erc20_bridge_address);
        self.owner.write(initial_owner);
    }


    #[abi(embed_v0)]
    impl MadaraFactoryImpl of IMadaraFactory<ContractState> {
        fn deploy_bridges(
            ref self: ContractState,
        ) -> (ContractAddress, ContractAddress, ContractAddress) {
            // Deploy l2 eth bridge
            let provisional_gov_admin = starknet::get_contract_address();
            let mut calldata = ArrayTrait::new();
            provisional_gov_admin.serialize(ref calldata);
            0.serialize(ref calldata); // upgrade_delay

            let (l2_eth_bridge, _) = deploy_syscall(
                self.token_bridge_class_hash.read(),
                'Eth_bridge_salt'.into(),
                calldata.span(),
                false,
            )
                .unwrap_syscall();

            // Deploy Eth
            let mut calldata = ArrayTrait::new();
            'Ether'.serialize(ref calldata);
            'Eth'.serialize(ref calldata);
            18.serialize(ref calldata);
            0.serialize(ref calldata);
            0.serialize(ref calldata);
            l2_eth_bridge.serialize(ref calldata); // permitted_minter
            self.owner.read().serialize(ref calldata); // l2_token_governance
            0.serialize(ref calldata); // DEFAULT_UPGRADE_DELAY

            let (l2_eth_token, _) = deploy_syscall(
                self.erc20_class_hash.read(), 'Eth_salt'.into(), calldata.span(), false,
            )
                .unwrap_syscall();

            let ethBridgeRole = IRolesDispatcher { contract_address: l2_eth_bridge };
            // Start upgrading eth bridge
            ethBridgeRole.register_upgrade_governor(provisional_gov_admin);

            let ethBridgeReplaceable = IReplaceableDispatcher { contract_address: l2_eth_bridge };
            // Add implementation
            let impl_data: ImplementationData = ImplementationData {
                impl_hash: self.token_bridge_class_hash.read(),
                eic_data: Some(
                    EICData {
                        eic_hash: self.eic_class_hash.read(),
                        eic_init_data: ['eth', l2_eth_token.into()].span(),
                    },
                ),
                final: false,
            };
            ethBridgeReplaceable.add_new_implementation(impl_data);
            ethBridgeReplaceable.replace_to(impl_data);

            ethBridgeRole.remove_upgrade_governor(provisional_gov_admin);
            // Ensure to remove this constract as the provisional_gov post the deployment setup
            ethBridgeRole.register_governance_admin(self.owner.read());

            // Deploy Token Bridge
            let provisional_gov_admin = starknet::get_contract_address();
            let mut calldata = ArrayTrait::new();
            provisional_gov_admin.serialize(ref calldata);
            0.serialize(ref calldata); // upgrade_delay

            let (l2_token_bridge, _) = deploy_syscall(
                self.token_bridge_class_hash.read(),
                'Token_bridge_salt'.into(),
                calldata.span(),
                false,
            )
                .unwrap_syscall();

            (l2_eth_token, l2_eth_bridge, l2_token_bridge)
        }
    }
}

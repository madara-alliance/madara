/// Simple contract for managing balance.
#[starknet::contract]
mod MadaraFactory {
    use core::array::ArrayTrait;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::syscalls::deploy_syscall;
    use starknet::{ClassHash, ContractAddress, EthAddress, SyscallResultTrait};
    use crate::interfaces::bridge::{ITokenBridgeAdminDispatcher, ITokenBridgeAdminDispatcherTrait};
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

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        ContractsDeployed: ContractsDeployed,
    }

    #[derive(Drop, starknet::Event)]
    struct ContractsDeployed {
        l2_eth_token: ContractAddress,
        l2_eth_bridge: ContractAddress,
        l2_token_bridge: ContractAddress,
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
            // Deploy Eth Bridge
            let l2_eth_bridge = self.deploy_eth_bridge();

            // Deploy Eth Token
            let l2_eth_token = self.deploy_eth_token(l2_eth_bridge);

            let eth_bridge_role = IRolesDispatcher { contract_address: l2_eth_bridge };
            // Start upgrading eth bridge
            eth_bridge_role.register_upgrade_governor(starknet::get_contract_address());

            let eth_bridge_replaceable = IReplaceableDispatcher { contract_address: l2_eth_bridge };
            // Add implementation
            let impl_data: ImplementationData = ImplementationData {
                impl_hash: self.token_bridge_class_hash.read(),
                eic_data: Some(
                    EICData {
                        eic_hash: self.eic_class_hash.read(),
                        // value of literal 'eth' is 0x657468
                        // This will be used in place of the address of the token on the L1,
                        //as Eth does not have a token address on L1.
                        eic_init_data: ['eth', l2_eth_token.into()].span(),
                    },
                ),
                final: false,
            };
            eth_bridge_replaceable.add_new_implementation(impl_data);
            eth_bridge_replaceable.replace_to(impl_data);

            eth_bridge_role.remove_upgrade_governor(starknet::get_contract_address());
            // Ensure to remove this constract as the provisional_gov post the deployment setup
            eth_bridge_role.register_governance_admin(self.owner.read());

            // Deploy Token Bridge
            let l2_token_bridge = self.deploy_token_bridge();

            // Emit event with all deployed contract addresses
            self.emit(Event::ContractsDeployed(ContractsDeployed {
                l2_eth_token,
                l2_eth_bridge,
                l2_token_bridge,
            }));

            (l2_eth_token, l2_eth_bridge, l2_token_bridge)
        }

        fn deploy_token_bridge(ref self: ContractState) -> ContractAddress {
            // Deploy Token Bridge
            let provisional_gov_admin = starknet::get_contract_address();
            let mut calldata = ArrayTrait::new();
            provisional_gov_admin.serialize(ref calldata);
            0.serialize(ref calldata); // upgrade_delay

            let (l2_token_bridge, _) = deploy_syscall(
                self.token_bridge_class_hash.read(),
                'Token_bridge_salt'.into(),
                calldata.span(),
                true,
            )
                .unwrap_syscall();
            // Configure the token bridge
            self.configure_bridge(l2_token_bridge, self.l1_erc20_bridge_address.read(), true);

            l2_token_bridge
        }

        fn deploy_eth_bridge(ref self: ContractState) -> ContractAddress {
           // Deploy l2 eth bridge
           let provisional_gov_admin = starknet::get_contract_address();

           // Creating the calldata to be passed to
           // the constructor of the EthBridge contract
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

           // Setting up the bridge with the correct permissions and configurations
           self.configure_bridge(l2_eth_bridge, self.l1_eth_bridge_address.read(), false);

           l2_eth_bridge
        }

        fn deploy_eth_token(ref self: ContractState, l2_eth_bridge: ContractAddress) -> ContractAddress {
            // Deploy Eth Token
            let mut calldata = ArrayTrait::new();
            'Ether'.serialize(ref calldata); // name
            'ETH'.serialize(ref calldata); // symbol
            18.serialize(ref calldata); // decimals
            0.serialize(ref calldata); // initial_supply
            0.serialize(ref calldata); // initial_supply_recipient
            l2_eth_bridge.serialize(ref calldata); // permitted_minter
            self.owner.read().serialize(ref calldata); // provisional_governance_admin
            0.serialize(ref calldata); // upgrade_delay

            let (l2_eth_token, _) = deploy_syscall(
                self.erc20_class_hash.read(), 'Eth_salt'.into(), calldata.span(), false,
            )
                .unwrap_syscall();

            l2_eth_token
        }
    }


    // Role                |   Role Admin
    // ----------------------------------------
    // GOVERNANCE_ADMIN    |   GOVERNANCE_ADMIN
    // UPGRADE_GOVERNOR    |   GOVERNANCE_ADMIN
    // APP_ROLE_ADMIN      |   GOVERNANCE_ADMIN
    // APP_GOVERNOR        |   APP_ROLE_ADMIN
    // OPERATOR            |   APP_ROLE_ADMIN
    // TOKEN_ADMIN         |   APP_ROLE_ADMIN
    // SECURITY_ADMIN      |   SECURITY_ADMIN
    // SECURITY_AGENT      |   SECURITY_ADMIN.
    #[generate_trait]
    impl InternalImpl of InternalTrait {
        // This function is used for
        // 1. setting the corresponding l1 bridge address using `set_l1_bridge`
        // 2. setting the l2 token governance address using `set_l2_token_governance`
        // 3. setting the erc20 class hash using `set_erc20_class_hash`
        // The setting are allowed only if you hold the app_governor role.
        // the app_governor role is granted only by the app_role_admin role.
        // the app_role_admin role is granted only by the governance_admin role.

        // These roles are later removed
        // by the `remove_app_governor` and `remove_app_role_admin` functions.
        // The governance_admin cannot be renounced,
        // and can only be revoked by another governance_admin.
        fn configure_bridge(
            ref self: ContractState,
            bridge_address: ContractAddress,
            l1_bridge: EthAddress,
            is_erc20: bool,
        ) {
            let contract = starknet::get_contract_address();
            let bridge_role = IRolesDispatcher { contract_address: bridge_address };
            let bridge_admin = ITokenBridgeAdminDispatcher { contract_address: bridge_address };
            // Registering the contract as an app role admin
            bridge_role.register_app_role_admin(contract);
            // Registering the contract as an app governor
            bridge_role.register_app_governor(contract);
            // Setting the l2 token governance address
            bridge_admin.set_l2_token_governance(self.owner.read());
            // Setting the corresponding l1 bridge address
            bridge_admin.set_l1_bridge(l1_bridge);
            if is_erc20 {
                bridge_admin.set_erc20_class_hash(self.erc20_class_hash.read());
            }
            // Removing the contract as an app governor
            bridge_role.remove_app_governor(contract);
            // Removing the contract as an app role admin
            bridge_role.remove_app_role_admin(contract);
            // Registering the contract as a governance admin
            bridge_role.register_governance_admin(self.owner.read());
        }
    }
}

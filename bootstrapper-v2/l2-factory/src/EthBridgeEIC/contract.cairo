// An EIC contract to set the bridge as legacy (vs. multi).
#[starknet::contract]
mod EIC {
    use core::num::traits::zero::Zero;
    use starknet::storage::{
        StorageMapReadAccess, StorageMapWriteAccess, StoragePointerReadAccess,
        StoragePointerWriteAccess,
    };
    use starknet::{ContractAddress, EthAddress};
    use crate::EthBridgeEIC::interface::IEICInitializable;


    #[storage]
    struct Storage {
        // --- Token Bridge ---
        // Mapping from between l1<->l2 token addresses.
        l1_l2_token_map: starknet::storage::Map<EthAddress, ContractAddress>,
        l2_l1_token_map: starknet::storage::Map<ContractAddress, EthAddress>,
        // `l2_token` is a legacy storage variable from older versions.
        // It should be written to as well to prevent multiple init, making the bridge a single.
        // and also to support legact L1-L2 msgs.
        l2_token: ContractAddress,
    }

    #[abi(embed_v0)]
    impl EICInitializableImpl of IEICInitializable<ContractState> {
        fn eic_initialize(ref self: ContractState, eic_init_data: Span<felt252>) {
            assert(eic_init_data.len() == 2, 'EIC_INIT_DATA_LEN_MISMATCH_2');
            let l1_token: EthAddress = (*eic_init_data[0]).try_into().unwrap();
            let l2_token: ContractAddress = (*eic_init_data[1]).try_into().unwrap();
            self.set_token_maps(:l1_token, :l2_token);
        }
    }

    #[generate_trait]
    impl InternalImpl of InternalTrait {
        fn set_token_maps(
            ref self: ContractState, l1_token: EthAddress, l2_token: ContractAddress,
        ) {
            assert(self.l2_token.read().is_zero(), 'L2 bridge initialized');
            assert(l1_token.is_non_zero(), 'Zero l1 token');
            assert(l2_token.is_non_zero(), 'Zero l2 token');

            assert(self.l1_l2_token_map.read(l1_token).is_zero(), 'L2 bridge initialized');
            assert(self.l2_l1_token_map.read(l2_token).is_zero(), 'L2 bridge initialized');

            self.l2_token.write(l2_token);
            self.l1_l2_token_map.write(l1_token, l2_token);
            self.l2_l1_token_map.write(l2_token, l1_token);
        }
    }
}


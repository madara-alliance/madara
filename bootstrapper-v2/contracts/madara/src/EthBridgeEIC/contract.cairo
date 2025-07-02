// An EIC contract to set the bridge as legacy (vs. multi).
//
// # Context
// 1. Starkgate has 2 versions of Bridge Contracts
// [Legacy](https://github.com/starknet-io/starkgate-contracts/blob/cairo-1/src/solidity/LegacyBridge.sol#L12)
// and the newer(latest)
// [MultiBridge](https://github.com/starknet-io/starkgate-contracts/blob/cairo-1/src/solidity/StarknetTokenBridge.sol#L24)
//
// 2. The legacy bridge suppports one token per contract, while the Multi
// bridge supports multiple tokens for the same contract Starknet has upgraded the contract with
// the LegacyBridge code with the new TokenBridge as well.
//
// # Aim
// 3. While deploying we want to directly deploy the latest version instead of deploying Legacy
// first and then the upgrading them, as the initial Legacy bridge were in cairo 0, which cant be
// declared/deployed post starknet 0.14.0
//
// # Approach
// 4. Due to differences of nature of Eth token with other ERC20 on Ethereum, there are 2 different
// Contracts for EthBridge and TokenBridge(Starkgate overrides the existing functions like `deposit`
// and `acceptDeposit`)
//
// 5. Also `token_bridge.cairo` strores the `l1_bridge` which can have only single value(obviously)
//
// 6. Combining 4. and 5. mandates that we will have to have 2 instances of `token_bridge.cairo` on
// on L2 as well.
//
// 7. The EthBridge has the `enroll_token` disabled, while uses the `bridgeToken()` which can be set
// using the EIC 8. The `l1_handler` for `deposit` on `token_bridge.cairo` will need to have the
// maps `l1_l2_token_map` and `l2_l1_token_map` being set (for Eth in this case)
//
// 8. Because of 7. this can't be done with existing flows, we will have to uuse an EIC to set them
// during deployment. Also it will be good that we set `l2_token` although not strictly needed, as
// this is how the corresponding bridge on L1 assumes it to work
//
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


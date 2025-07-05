#[starknet::interface]
pub trait IPiltover<TContractState> {
    fn update_state(
        ref self: TContractState,
        snos_output: Span<felt252>,
        program_output: Span<felt252>,
        onchain_data_hash: felt252,
        onchain_data_size: u256
    );

    fn get_is_updated(self: @TContractState, onchain_data_hash: felt252) -> bool;
}

#[starknet::contract]
mod Piltover {
    use starknet::storage::Map;
    #[storage]
    struct Storage {
        is_updated: Map<felt252, bool>,
    }

    #[abi(embed_v0)]
    impl IPiltoverImpl of super::IPiltover<ContractState> {
        fn update_state(
            ref self: ContractState,
            snos_output: Span<felt252>,
            program_output: Span<felt252>,
            onchain_data_hash: felt252,
            onchain_data_size: u256
        ) {
            self.is_updated.write(onchain_data_hash, true);
        }

        fn get_is_updated(self: @ContractState, onchain_data_hash: felt252) -> bool {
            self.is_updated.read(onchain_data_hash)
        }
    }
}

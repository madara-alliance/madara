use starknet::{ClassHash, ContractAddress};

#[starknet::interface]
pub trait IDeployer<TState> {
    fn deploy_contract(
        ref self: TState,
        class_hash: ClassHash,
        salt: felt252,
        from_zero: bool,
        calldata: Span<felt252>,
    ) -> ContractAddress;
}


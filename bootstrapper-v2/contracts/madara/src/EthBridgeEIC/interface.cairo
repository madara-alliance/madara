#[starknet::interface]
pub trait IEICInitializable<TContractState> {
    fn eic_initialize(ref self: TContractState, eic_init_data: Span<felt252>);
}

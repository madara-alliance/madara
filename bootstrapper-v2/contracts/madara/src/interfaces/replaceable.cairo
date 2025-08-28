use starknet::class_hash::ClassHash;

// Holds EIC data.
// * eic_hash is the EIC class hash.
// * eic_init_data is a span of the EIC init args.
#[derive(Copy, Drop, Serde, PartialEq)]
pub struct EICData {
    pub eic_hash: ClassHash,
    pub eic_init_data: Span<felt252>,
}

// Holds implementation data.
// * impl_hash is the implementation class hash.
// * eic_data is the EIC data when applicable, and empty otherwise.
// * final indicates whether the implementation is finalized.
#[derive(Copy, Drop, Serde, PartialEq)]
pub struct ImplementationData {
    pub impl_hash: ClassHash,
    pub eic_data: Option<EICData>,
    pub final: bool,
}

#[starknet::interface]
pub trait IReplaceable<TContractState> {
    fn add_new_implementation(ref self: TContractState, implementation_data: ImplementationData);
    fn replace_to(ref self: TContractState, implementation_data: ImplementationData);
}


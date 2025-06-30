use starknet::{ClassHash, ContractAddress, EthAddress};

#[starknet::interface]
pub trait ITokenBridgeAdmin<TContractState> {
    fn get_erc20_class_hash(self: @TContractState) -> ClassHash;
    fn get_l2_token_governance(self: @TContractState) -> ContractAddress;
    fn set_l1_bridge(ref self: TContractState, l1_bridge_address: EthAddress);
    fn set_erc20_class_hash(ref self: TContractState, erc20_class_hash: ClassHash);
    fn set_l2_token_governance(ref self: TContractState, l2_token_governance: ContractAddress);
    fn enable_withdrawal_limit(ref self: TContractState, l1_token: ContractAddress);
    fn disable_withdrawal_limit(ref self: TContractState, l1_token: ContractAddress);
}

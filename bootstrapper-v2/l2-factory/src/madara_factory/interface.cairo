use starknet::ContractAddress;

#[starknet::interface]
pub trait IMadaraFactory<TContractState> {
    // Returns EthAddress, EthBridgeAddress, TokenBridgeAddress
    fn deploy_bridges(
        ref self: TContractState,
    ) -> (ContractAddress, ContractAddress, ContractAddress);
}

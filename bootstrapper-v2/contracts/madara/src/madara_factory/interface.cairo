use starknet::ContractAddress;

#[starknet::interface]
pub trait IMadaraFactory<TContractState> {
    // Returns EthAddress, EthBridgeAddress, TokenBridgeAddress
    fn deploy_bridges(
        ref self: TContractState,
    ) -> (ContractAddress, ContractAddress, ContractAddress, ContractAddress);

    fn deploy_token_bridge(
        ref self: TContractState,
    ) -> ContractAddress;

    fn deploy_eth_bridge(
        ref self: TContractState,
    ) -> ContractAddress;

    fn deploy_eth_token(
        ref self: TContractState, l2_eth_bridge: ContractAddress
    ) -> ContractAddress;

    fn deploy_universal_deployer(
        ref self: TContractState,
    ) -> ContractAddress;
}

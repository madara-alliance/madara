#[starknet::contract]
mod StateUpdateContract {
    type StateRoot = felt252;
    type BlockNumber = felt252;
    type BlockHash = felt252;

    #[storage]
    struct Storage {
        state_root: StateRoot,
        block_number: BlockNumber,
        block_hash: BlockHash,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        LogStateUpdate: LogStateUpdate
    }

    #[derive(Drop, starknet::Event)]
    struct LogStateUpdate {
        state_root: felt252,
        block_number: felt252,
        block_hash: felt252,
    }

    #[constructor]
    fn constructor(ref self: ContractState) {
        // Initialize with default values
        self.state_root.write(0);
        self.block_number.write(0);
        self.block_hash.write(0);
    }

    #[external(v0)]
    fn update_state(
        ref self: ContractState,
        block_number: BlockNumber,
        state_root: StateRoot,
        block_hash: BlockHash
    ) {
        self.state_root.write(state_root);
        self.block_number.write(block_number);
        self.block_hash.write(block_hash);

        self.emit(LogStateUpdate { state_root, block_number, block_hash, });
    }

    #[external(v0)]
    fn get_state(self: @ContractState) -> (StateRoot, BlockNumber, BlockHash) {
        (self.state_root.read(), self.block_number.read(), self.block_hash.read())
    }
}

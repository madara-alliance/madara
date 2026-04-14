#[starknet::contract]
mod DeclareTestContract {
    #[storage]
    struct Storage {
        value: felt252,
    }

    #[constructor]
    fn constructor(ref self: ContractState) {
        self.value.write(0);
    }

    #[external(v0)]
    fn set_value(ref self: ContractState, new_value: felt252) {
        self.value.write(new_value);
    }

    #[external(v0)]
    fn get_value(self: @ContractState) -> felt252 {
        self.value.read()
    }
}

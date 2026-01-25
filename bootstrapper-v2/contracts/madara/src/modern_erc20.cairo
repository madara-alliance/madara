use starknet::ContractAddress;

#[starknet::interface]
pub trait IModernERC20<TContractState> {
    fn name(self: @TContractState) -> felt252;
    fn symbol(self: @TContractState) -> felt252;
    fn decimals(self: @TContractState) -> u8;
    fn total_supply(self: @TContractState) -> u256;
    fn totalSupply(self: @TContractState) -> u256;  // camelCase alias
    fn balance_of(self: @TContractState, account: ContractAddress) -> u256;
    fn balanceOf(self: @TContractState, account: ContractAddress) -> u256;  // camelCase alias
    fn allowance(self: @TContractState, owner: ContractAddress, spender: ContractAddress) -> u256;
    fn transfer(ref self: TContractState, recipient: ContractAddress, amount: u256) -> bool;
    fn transfer_from(
        ref self: TContractState,
        sender: ContractAddress,
        recipient: ContractAddress,
        amount: u256
    ) -> bool;
    fn transferFrom(
        ref self: TContractState,
        sender: ContractAddress,
        recipient: ContractAddress,
        amount: u256
    ) -> bool;  // camelCase alias
    fn approve(ref self: TContractState, spender: ContractAddress, amount: u256) -> bool;
    fn permissioned_mint(ref self: TContractState, recipient: ContractAddress, amount: u256);
    fn permissioned_burn(ref self: TContractState, account: ContractAddress, amount: u256);
    fn get_permitted_minter(self: @TContractState) -> ContractAddress;
    fn get_governance_admin(self: @TContractState) -> ContractAddress;
}

#[starknet::contract]
pub mod ModernERC20 {
    use starknet::ContractAddress;
    use starknet::get_caller_address;
    use starknet::storage::{Map, StoragePointerReadAccess, StoragePointerWriteAccess, StorageMapReadAccess, StorageMapWriteAccess};

    #[storage]
    struct Storage {
        name: felt252,
        symbol: felt252,
        decimals: u8,
        total_supply: u256,
        ERC20_balances: Map<ContractAddress, u256>,
        allowances: Map<(ContractAddress, ContractAddress), u256>,
        permitted_minter: ContractAddress,
        governance_admin: ContractAddress,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        Transfer: Transfer,
        Approval: Approval,
    }

    #[derive(Drop, starknet::Event)]
    struct Transfer {
        #[key]
        from: ContractAddress,
        #[key]
        to: ContractAddress,
        value: u256,
    }

    #[derive(Drop, starknet::Event)]
    struct Approval {
        #[key]
        owner: ContractAddress,
        #[key]
        spender: ContractAddress,
        value: u256,
    }

    // Constructor matching StarkGate ERC20 signature
    #[constructor]
    fn constructor(
        ref self: ContractState,
        name: felt252,
        symbol: felt252,
        decimals: u8,
        initial_supply: u256,
        recipient: ContractAddress,
        permitted_minter: ContractAddress,
        provisional_governance_admin: ContractAddress,
        upgrade_delay: felt252, // Ignored for simplicity
    ) {
        self.name.write(name);
        self.symbol.write(symbol);
        self.decimals.write(decimals);
        self.permitted_minter.write(permitted_minter);
        self.governance_admin.write(provisional_governance_admin);

        // Mint initial supply to recipient
        if initial_supply > 0 {
            self.ERC20_balances.write(recipient, initial_supply);
            self.total_supply.write(initial_supply);

            let zero_address: ContractAddress = 0.try_into().unwrap();
            self.emit(Transfer { from: zero_address, to: recipient, value: initial_supply });
        }
    }

    #[abi(embed_v0)]
    impl ModernERC20Impl of super::IModernERC20<ContractState> {
        fn name(self: @ContractState) -> felt252 {
            self.name.read()
        }

        fn symbol(self: @ContractState) -> felt252 {
            self.symbol.read()
        }

        fn decimals(self: @ContractState) -> u8 {
            self.decimals.read()
        }

        fn total_supply(self: @ContractState) -> u256 {
            self.total_supply.read()
        }

        fn balance_of(self: @ContractState, account: ContractAddress) -> u256 {
            self.ERC20_balances.read(account)
        }

        fn allowance(self: @ContractState, owner: ContractAddress, spender: ContractAddress) -> u256 {
            self.allowances.read((owner, spender))
        }

        fn transfer(ref self: ContractState, recipient: ContractAddress, amount: u256) -> bool {
            let sender = get_caller_address();
            self._transfer(sender, recipient, amount);
            true
        }

        fn transfer_from(
            ref self: ContractState,
            sender: ContractAddress,
            recipient: ContractAddress,
            amount: u256
        ) -> bool {
            let caller = get_caller_address();
            let current_allowance = self.allowances.read((sender, caller));
            assert(current_allowance >= amount, 'Insufficient allowance');
            self.allowances.write((sender, caller), current_allowance - amount);
            self._transfer(sender, recipient, amount);
            true
        }

        fn approve(ref self: ContractState, spender: ContractAddress, amount: u256) -> bool {
            let caller = get_caller_address();
            self.allowances.write((caller, spender), amount);
            self.emit(Approval { owner: caller, spender, value: amount });
            true
        }

        // Permissioned mint - only permitted_minter can call
        fn permissioned_mint(ref self: ContractState, recipient: ContractAddress, amount: u256) {
            let caller = get_caller_address();
            assert(caller == self.permitted_minter.read(), 'Only minter can mint');

            let new_balance = self.ERC20_balances.read(recipient) + amount;
            self.ERC20_balances.write(recipient, new_balance);
            let new_total = self.total_supply.read() + amount;
            self.total_supply.write(new_total);

            let zero_address: ContractAddress = 0.try_into().unwrap();
            self.emit(Transfer { from: zero_address, to: recipient, value: amount });
        }

        // Permissioned burn
        fn permissioned_burn(ref self: ContractState, account: ContractAddress, amount: u256) {
            let caller = get_caller_address();
            assert(caller == self.permitted_minter.read(), 'Only minter can burn');

            let balance = self.ERC20_balances.read(account);
            assert(balance >= amount, 'Insufficient balance');
            self.ERC20_balances.write(account, balance - amount);
            let new_total = self.total_supply.read() - amount;
            self.total_supply.write(new_total);

            let zero_address: ContractAddress = 0.try_into().unwrap();
            self.emit(Transfer { from: account, to: zero_address, value: amount });
        }

        fn get_permitted_minter(self: @ContractState) -> ContractAddress {
            self.permitted_minter.read()
        }

        fn get_governance_admin(self: @ContractState) -> ContractAddress {
            self.governance_admin.read()
        }

        // camelCase aliases for compatibility with Madara fee validation
        fn totalSupply(self: @ContractState) -> u256 {
            self.total_supply()
        }

        fn balanceOf(self: @ContractState, account: ContractAddress) -> u256 {
            self.balance_of(account)
        }

        fn transferFrom(
            ref self: ContractState,
            sender: ContractAddress,
            recipient: ContractAddress,
            amount: u256
        ) -> bool {
            self.transfer_from(sender, recipient, amount)
        }
    }

    #[generate_trait]
    impl InternalImpl of InternalTrait {
        fn _transfer(
            ref self: ContractState,
            sender: ContractAddress,
            recipient: ContractAddress,
            amount: u256
        ) {
            let sender_balance = self.ERC20_balances.read(sender);
            assert(sender_balance >= amount, 'Insufficient balance');
            self.ERC20_balances.write(sender, sender_balance - amount);
            let recipient_balance = self.ERC20_balances.read(recipient);
            self.ERC20_balances.write(recipient, recipient_balance + amount);
            self.emit(Transfer { from: sender, to: recipient, value: amount });
        }
    }
}

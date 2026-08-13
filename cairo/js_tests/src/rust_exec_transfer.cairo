// Devnet-only Rust Exec E2E fixture. This is not production contract support.
use starknet::ContractAddress;

#[starknet::interface]
pub trait IRustExecTransfer<TContractState> {
    fn transfer(ref self: TContractState, recipient: ContractAddress, amount: felt252) -> bool;
    fn transfer_with_comparator_mismatch(
        ref self: TContractState, recipient: ContractAddress, amount: felt252,
    ) -> bool;
    fn get_last_transfer(
        self: @TContractState
    ) -> (ContractAddress, ContractAddress, felt252, felt252);
}

#[starknet::contract]
pub mod RustExecTransfer {
    use starknet::{ContractAddress, get_caller_address};
    use super::IRustExecTransfer;

    #[storage]
    struct Storage {
        last_sender: ContractAddress,
        last_recipient: ContractAddress,
        last_amount: felt252,
        transfer_count: felt252,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        Transfer: Transfer,
    }

    #[derive(Drop, starknet::Event)]
    struct Transfer {
        #[key]
        from: ContractAddress,
        #[key]
        to: ContractAddress,
        amount: felt252,
    }

    #[abi(embed_v0)]
    impl RustExecTransferImpl of IRustExecTransfer<ContractState> {
        fn transfer(ref self: ContractState, recipient: ContractAddress, amount: felt252) -> bool {
            record_transfer(ref self, recipient, amount)
        }

        /// E2E fixture whose Cairo behavior is intentionally identical to `transfer`.
        fn transfer_with_comparator_mismatch(
            ref self: ContractState, recipient: ContractAddress, amount: felt252,
        ) -> bool {
            record_transfer(ref self, recipient, amount)
        }

        fn get_last_transfer(
            self: @ContractState
        ) -> (ContractAddress, ContractAddress, felt252, felt252) {
            (
                self.last_sender.read(),
                self.last_recipient.read(),
                self.last_amount.read(),
                self.transfer_count.read(),
            )
        }
    }

    fn record_transfer(
        ref self: ContractState, recipient: ContractAddress, amount: felt252
    ) -> bool {
        let sender = get_caller_address();
        self.last_sender.write(sender);
        self.last_recipient.write(recipient);
        self.last_amount.write(amount);
        self.transfer_count.write(self.transfer_count.read() + 1);
        self.emit(Transfer { from: sender, to: recipient, amount });
        true
    }
}

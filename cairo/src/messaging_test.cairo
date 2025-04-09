use starknet::ContractAddress;
use core::integer::u256;

#[derive(Drop, Serde)]
struct MessageData {
    from_address: ContractAddress,
    to_address: felt252,
    selector: felt252,
    payload: Array<felt252>,
    nonce: felt252,
}

// Separate storage struct to handle the array storage properly
#[derive(Drop, Serde, starknet::Store)]
struct StorageMessageData {
    from_address: ContractAddress,
    to_address: felt252,
    selector: felt252,
    nonce: felt252,
}

#[starknet::interface]
trait IMessagingContract<TContractState> {
    fn get_message_data(self: @TContractState) -> MessageData;
    fn fire_event(ref self: TContractState);
    fn sn_to_appchain_messages(self: @TContractState, msg_hash: felt252) -> felt252;
    fn set_is_canceled(ref self: TContractState, value: bool);
    fn get_l1_to_l2_msg_hash(self: @TContractState) -> felt252;
}

#[starknet::contract]
mod MessagingContract {
    use super::IMessagingContract;
    use super::MessageData;
    use core::array::SpanTrait;
    use core::option::OptionTrait;
    use core::traits::Into;
    use starknet::ContractAddress;
    use core::array::ArrayTrait;
    use core::poseidon::poseidon_hash_span;
    use core::integer::u256;

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        MessageSent: MessageSent,
    }

    #[derive(Drop, starknet::Event)]
    struct MessageSent {
        #[key]
        message_hash: felt252,
        #[key]
        from_address: ContractAddress,
        #[key]
        to_address: felt252,
        selector: felt252,
        nonce: felt252,
        payload: Array<felt252>,
    }

    #[storage]
    struct Storage {
        is_canceled: bool
    }

    #[abi(embed_v0)]
    impl MessagingContract of super::IMessagingContract<ContractState> {
        fn get_message_data(self: @ContractState) -> MessageData {
            let mut payload: Array<felt252> = ArrayTrait::new();
            // Transfer L2 ---> L3
            // from_address : <madara devnet address #10>
            // to_address : <ETH address>
            // selector : transfer
            // payload : [ <madara devnet address #9> , 30 ETH ]
            payload
                .append(
                    1745450722268439108899567493174320056804647958314420522290024379112230030194
                        .into()
                );
            payload.append(30000000000000000000.into());
            payload.append(0.into());

            MessageData {
                from_address: 3293945099482077566294620753663887236810230524774221047563633702975851058323
                    .try_into()
                    .unwrap(),
                to_address: 2087021424722619777119509474943472645767659996348769578120564519014510906823
                    .into(),
                selector: 232670485425082704932579856502088130646006032362877466777181098476241604910
                    .into(),
                payload,
                nonce: 0.into(),
            }
        }

        fn fire_event(ref self: ContractState) {
            let data = self.get_message_data();
            let hash = self.get_l1_to_l2_msg_hash();
            self
                .emit(
                    Event::MessageSent(
                        MessageSent {
                            message_hash: hash,
                            from_address: data.from_address,
                            to_address: data.to_address,
                            selector: data.selector,
                            payload: data.payload,
                            nonce: data.nonce,
                        }
                    )
                );
        }

        // here is the output of the call:
        // https://github.com/keep-starknet-strange/piltover/blob/161cb3f66d256e4d1211c6b50e5d353afb713a3e/src/messaging/types.cairo#L5
        // pub enum MessageToAppchainStatus {
        //     #[default]
        //     NotSent,
        //     Sealed,
        //     Cancelled,
        //     Pending: Nonce
        // }
        // so we are return 2 when the message is cancelled and 1 when it is not
        fn sn_to_appchain_messages(self: @ContractState, msg_hash: felt252) -> felt252 {
            if self.is_canceled.read() {
                2.into()
            } else {
                1.into()
            }
        }

        fn set_is_canceled(ref self: ContractState, value: bool) {
            self.is_canceled.write(value);
        }

        fn get_l1_to_l2_msg_hash(self: @ContractState) -> felt252 {
            let data = self.get_message_data();
            let mut hash_data: Array<felt252> = ArrayTrait::new();
            hash_data.append(data.from_address.into());
            hash_data.append(data.to_address);
            hash_data.append(data.selector);
            hash_data.append(data.nonce);
            let len: felt252 = data.payload.len().into();
            hash_data.append(len);

            let mut i: usize = 0;
            let payload_span = data.payload.span();
            loop {
                if i >= data.payload.len() {
                    break;
                }
                let value = *payload_span.at(i);
                let value_felt: felt252 = value.try_into().unwrap();
                hash_data.append(value_felt);
                i += 1;
            };

            poseidon_hash_span(hash_data.span())
        }
    }
}

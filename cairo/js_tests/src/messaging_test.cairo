use starknet::ContractAddress;

#[derive(Drop, Serde)]
struct MessageData {
    from: ContractAddress,
    to: felt252,
    selector: felt252,
    payload: Array<felt252>,
    nonce: felt252,
}

// Separate storage struct to handle the array storage properly
#[derive(Drop, Serde, starknet::Store)]
struct StorageMessageData {
    from: ContractAddress,
    to: felt252,
    selector: felt252,
    nonce: felt252,
}

pub type Nonce = felt252;

#[derive(Serde, Drop, PartialEq, starknet::Store, Default)]
pub enum MessageToAppchainStatus {
    #[default]
    NotSent,
    Sealed,
    Cancelled,
    Pending: Nonce, // sn->appc: The nonce > 0.
}

#[starknet::interface]
trait IMessagingContract<TContractState> {
    fn get_message_data(self: @TContractState) -> MessageData;
    fn fire_event(ref self: TContractState);
    fn sn_to_appchain_messages(self: @TContractState, msg_hash: felt252) -> MessageToAppchainStatus;
    fn cancel_event(ref self: TContractState);
    fn get_l1_to_l2_msg_hash(self: @TContractState) -> felt252;
}

#[starknet::contract]
mod MessagingContract {
    use super::IMessagingContract;
    use super::{MessageData, MessageToAppchainStatus};
    use core::array::SpanTrait;
    use core::option::OptionTrait;
    use core::traits::Into;
    use starknet::ContractAddress;
    use core::array::ArrayTrait;
    use core::poseidon::poseidon_hash_span;

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        MessageSent: MessageSent,
        MessageCancellationStarted: MessageCancellationStarted,
    }

    #[derive(Drop, starknet::Event)]
    struct MessageSent {
        #[key]
        message_hash: felt252,
        #[key]
        from: ContractAddress,
        #[key]
        to: felt252,
        selector: felt252,
        nonce: felt252,
        payload: Array<felt252>,
    }
    #[derive(Drop, starknet::Event)]
    struct MessageCancellationStarted {
        #[key]
        message_hash: felt252,
        #[key]
        from: ContractAddress,
        #[key]
        to: felt252,
        selector: felt252,
        payload: Span<felt252>,
        nonce: felt252,
    }

    #[storage]
    struct Storage {
        is_canceled: bool
    }

    #[abi(embed_v0)]
    impl MessagingContract of super::IMessagingContract<ContractState> {
        // data taken from: https://sepolia.voyager.online/event/604902_2_0
        fn get_message_data(self: @ContractState) -> MessageData {
            let mut payload: Array<felt252> = ArrayTrait::new();
            // Transfer L2 ---> L3
            // from : <madara devnet address #10>
            // to : <ETH address>
            // selector : transfer
            // payload : [ <madara devnet address #9> , 30 ETH ]
            payload
                .append(
                    196052332073534950981109812936197997210225762447282671667548285438697329327
                        .into()
                );
            payload.append(0.into());
            payload.append(86374027119666973454853486.into());
            payload.append(11.into());
            payload.append(0.into());
            payload.append(4674116.into());
            payload.append(3.into());
            payload.append(18.into());

            MessageData {
                from: 116928254081234419462378178529324579584489887065328772660517990334005757698
                    .try_into()
                    .unwrap(),
                to: 254321392966061410989098621660241596001676783615828345549730562518690224743
                    .into(),
                selector: 1737780302748468118210503507461757847859991634169290761669750067796330642876
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
                            from: data.from,
                            to: data.to,
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
        // so we are returning 2 when the message is cancelled and 3 when it is not
        fn sn_to_appchain_messages(
            self: @ContractState, msg_hash: felt252
        ) -> MessageToAppchainStatus {
            if self.is_canceled.read() {
                MessageToAppchainStatus::Cancelled
            } else {
                MessageToAppchainStatus::Pending(0.into())
            }
        }

        fn cancel_event(ref self: ContractState) {
            self.is_canceled.write(true);
            let data = self.get_message_data();
            let hash = self.get_l1_to_l2_msg_hash();
            self
                .emit(
                    Event::MessageCancellationStarted(
                        MessageCancellationStarted {
                            message_hash: hash,
                            from: data.from,
                            to: data.to,
                            selector: data.selector,
                            payload: data.payload.span(),
                            nonce: data.nonce,
                        }
                    )
                );
        }

        fn get_l1_to_l2_msg_hash(self: @ContractState) -> felt252 {
            let data = self.get_message_data();
            let mut hash_data: Array<felt252> = ArrayTrait::new();
            hash_data.append(data.from.into());
            hash_data.append(data.to);
            hash_data.append(data.nonce);
            hash_data.append(data.selector);
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

# L1<>L2 messaging

- [X] Create a stream of LogMessageToL2 events
- [X] Get last synced block from Messaging DB
- [X] Consume the stream and log event
- [ ] Process message 
    - [X] Parse tx fee
    - [X] Parse transaction from event
    - [X] Check if message has already been processed
    - [X] Build transaction
    - [Waiting for mempool] Submit tx to mempool
    - [X] Update Messaging DB
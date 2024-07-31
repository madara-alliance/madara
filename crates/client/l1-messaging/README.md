# L1<>L2 messaging

- [ ] Create a stream of LogMessageToL2 events
- [ ] Get last synced block from Messaging DB
- [ ] Consume the stream and log event
- [ ] Process message 
    - [ ] Parse tx fee
    - [ ] Parse transaction from event
    - [ ] Check if message has already been processed
    - [ ] Build transaction
    - [ ] Submit tx to mempool
    - [ ] Update Messaging DB
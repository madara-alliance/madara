# L1<>L2 messaging

A few notes on the current design:

- There is no guarantee that L1->L2 messages will be executed in a centralized context.
- There is no ordering guarantee, nonces are solely used

- [x] Create a stream of LogMessageToL2 events
- [x] Get last synced block from Messaging DB
- [x] Consume the stream and log event
- [ ] Process message
  - [x] Parse tx fee
  - [x] Parse transaction from event
  - [x] Check if message has already been processed
  - [x] Build transaction
  - [Waiting for mempool] Submit tx to mempool
  - [x] Update Messaging DB
- [x] Handle Message Cancellation

## Tests

- E2E test #1
  - Launch Anvil Node
  - Launch Worker
  - Send L1->L2 message
  - Assert that event is emitted on L1
  - Assert that even was caught by the worker with correct data
  - Assert the tx hash computed by the worker is correct
  - Assert that the tx has been included in the mempool
  - Assert that DB was correctly updated (last synced block & nonce)
  - Assert that the tx was correctly executed

- E2E test #2
  - Should fail if we try to send multiple messages with same nonces

- E2E test #3
  - Message Cancellation

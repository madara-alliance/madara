# Madara Sync Architecture

 The sync system in Madara is responsible for synchronizing blockchain data from a sequencer/gateway to fullnodes. Here's how it works:

 **Core Components**

 1. **SyncController** (sync.rs:98-267): The main orchestrator that manages the entire sync process.
It coordinates between different pipeline stages and controls the sync flow.
 2. Three-Stage Pipeline Architecture:
   - Blocks Pipeline: Fetches blocks and state updates from the gateway
   - Classes Pipeline: Processes and stores contract classes
   - Apply State Pipeline: Applies state changes to the local database

 How It Works

 1. Gateway Provider: The sync connects to a Starknet gateway/sequencer (like the official Starknet gateway) using the GatewayProvider client.
 2. Parallel Processing: Each pipeline stage uses a PipelineController that:
   - Executes parallel steps for fetching data (concurrent block downloads)
   - Executes sequential steps for applying changes (ordered block processing)
   - Supports configurable parallelization (default 128 for blocks, 256 for classes)
 3. Block Processing Flow:
 Gateway → Fetch Block → Verify → Store Header/Txs → Process Classes → Apply State
 4. Key Features:
   - Probing: Continuously checks the gateway for new blocks (GatewayLatestProbe)
   - Pending Block Sync: Separately syncs pending blocks that haven't been finalized
   - Verification: Validates block hashes, transaction commitments, event commitments, and state diffs
   - Backpressure: Each pipeline stage can signal when it's ready for more work
 5. State Management:
   - Tracks sync progress through SyncMetrics
   - Maintains head status for blocks, transactions, events, and state diffs
   - Supports stopping at specific block heights
   - Can sync from any starting point
 6. Error Handling:
   - Validates block integrity before applying
   - Supports pre-v0.13.2 block format compatibility
   - Handles gateway errors gracefully

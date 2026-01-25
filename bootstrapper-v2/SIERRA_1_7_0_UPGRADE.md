# Sierra 1.7.0 ERC20 Fee Token Upgrade

## Summary

Successfully created a modern Sierra 1.7.0 compatible ERC20 contract to replace the legacy Sierra 1.3.0 fee token.

## What Was Changed

### 1. New ERC20 Contract Created
- **File**: `contracts/madara/src/modern_erc20.cairo`
- **Sierra Version**: 1.7.0 (Cairo 2.14.0)
- **Features**:
  - Full ERC20 standard implementation
  - Constructor compatible with StarkGate interface
  - Permissioned minting (only permitted_minter can mint)
  - Governance admin tracking

### 2. Artifacts Updated
- **Location**: `/Users/heemankverma/Work/Karnot/RvsC/madara/build-artifacts/starkgate_latest/`
- **Files**:
  - `modern_erc20.sierra.json` (Sierra 1.7.0)
  - `modern_erc20.casm.json`

### 3. Bootstrapper Constants Updated
- **File**: `src/setup/madara/constants.rs`
- **Change**: ERC20_SIERRA and ERC20_CASM now point to `modern_erc20.*` instead of `ERC20_070.*`

### 4. Library Module Updated
- **File**: `contracts/madara/src/lib.cairo`
- **Change**: Added `mod modern_erc20;`

## Verification

Sierra version confirmed:
```bash
jq '.sierra_program[0:3]' \
  build-artifacts/starkgate_latest/modern_erc20.sierra.json
# Output: ["0x1", "0x7", "0x0"] = Sierra 1.7.0
```

## How to Deploy

### Option 1: Automated Script

```bash
cd /Users/heemankverma/Work/Karnot/RvsC/madara/bootstrapper-v2

# Make script executable
chmod +x redeploy-with-modern-erc20.sh

# Run deployment
./redeploy-with-modern-erc20.sh
```

### Option 2: Manual Steps

#### Step 1: Stop Madara
```bash
pkill -f "madara.*--sequencer"
```

#### Step 2: Clean Database
```bash
cd /Users/heemankverma/Work/Karnot/RvsC/madara/madara
rm -rf db
```

#### Step 3: Start Madara
```bash
# In a separate terminal
cd /Users/heemankverma/Work/Karnot/RvsC/madara/madara
make run-madara-sequencer
```

#### Step 4: Deploy Infrastructure
```bash
cd /Users/heemankverma/Work/Karnot/RvsC/madara/bootstrapper-v2

RUST_LOG=info cargo run --release --bin bootstrapper-v2 -- \
  skip-base-layer \
  --madara-rpc-url http://localhost:9944 \
  --output-path output/madara_addresses.json
```

#### Step 5: Verify Fee Token Sierra Version
```bash
# Get fee token address
FEE_TOKEN=$(jq -r '.addresses.l2_eth_token' output/madara_addresses.json)

# Get class hash
CLASS_HASH=$(curl -s -X POST http://localhost:9944 \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getClassHashAt\",\"params\":{\"block_id\":\"latest\",\"contract_address\":\"$FEE_TOKEN\"},\"id\":1}" \
  | jq -r '.result')

# Check Sierra version (should show ["0x1", "0x7", "0x0"])
curl -s -X POST http://localhost:9944 \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getClass\",\"params\":{\"block_id\":\"latest\",\"class_hash\":\"$CLASS_HASH\"},\"id\":1}" \
  | jq '.result.sierra_program[0:3]'
```

#### Step 6: Redeploy Kamehameha Contracts
```bash
cd /Users/heemankverma/Work/Karnot/RvsC/madara/bootstrapper-v2

# Deploy each contract
for contract in SimpleCounter CounterWithEvent Random100Hashes MathBenchmark; do
  cargo run --release --bin deploy-contract $contract
  sleep 2
done
```

#### Step 7: Create User Account
```bash
# Generate a new private key or use existing one
PRIVATE_KEY="0x063bb0bb54374247449a9f6f978a3653b969e2a1294b35205d721ebc1539446a"

# Deploy account
cargo run --release --bin deploy-new-account $PRIVATE_KEY
```

#### Step 8: Transfer ETH to User Account
```bash
# Get the new account address from deploy output
USER_ACCOUNT="<address from deploy-new-account output>"

# Transfer 10,000 ETH
FEE_TOKEN=$(jq -r '.addresses.l2_eth_token' output/madara_addresses.json)

cargo run --release --bin transfer-eth \
  $FEE_TOKEN \
  $USER_ACCOUNT \
  0x21E19E0C9BAB2400000 \
  0x0
```

## Expected Results

After successful deployment, all contracts should be using Sierra 1.7.0:

1. **Fee Token (L2 ETH)**: Sierra 1.7.0 ✅
2. **Kamehameha Contracts**: Sierra 1.7.0 ✅
3. **User Account**: Sierra 1.7.0 ✅

## Comparison: Before vs After

### Before
- Fee Token: Sierra 1.3.0 (StarkGate ERC20_070)
- Kamehameha: Sierra 1.7.0
- **Issue**: Version mismatch

### After
- Fee Token: Sierra 1.7.0 (Modern ERC20)
- Kamehameha: Sierra 1.7.0
- **Status**: All Sierra 1.7.0 ✅

## Notes

- The Modern ERC20 maintains compatibility with StarkGate interface
- Constructor signature matches exactly what MadaraFactory expects
- All state from old deployment will be lost (fresh start)
- Bootstrap account will have 1 billion ETH after deployment

## Files Modified

1. `contracts/madara/src/modern_erc20.cairo` - New file
2. `contracts/madara/src/lib.cairo` - Added module
3. `src/setup/madara/constants.rs` - Updated artifact paths
4. `build-artifacts/starkgate_latest/modern_erc20.*` - New artifacts

## Rollback

To rollback to old ERC20:
```bash
cd /Users/heemankverma/Work/Karnot/RvsC/madara/bootstrapper-v2/src/setup/madara

# Edit constants.rs
# Change ERC20_SIERRA back to: "../build-artifacts/starkgate_latest/ERC20_070.sierra.json"
# Change ERC20_CASM back to: "../build-artifacts/starkgate_latest/ERC20_070.casm.json"

# Rebuild bootstrapper
cargo build --release --bin bootstrapper-v2

# Redeploy
```

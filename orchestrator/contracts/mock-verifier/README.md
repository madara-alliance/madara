# Mock GPS Verifier

Trust-free fact registry for mocknet / dev environments. Implements the `IFactRegistry`
interface that Starkware's Starknet core contract calls into to verify state-transition
facts. Our mock lets anyone register a fact; no proof is checked.

**Never deploy to an environment where proof validity matters.**

## Build

```bash
cd orchestrator/contracts/mock-verifier
forge build
```

Artifacts land in `out/MockGpsVerifier.sol/MockGpsVerifier.json`.

## Deploy (Sepolia example)

```bash
# Replace with your own RPC + signer
export ETH_RPC_URL=https://sepolia.infura.io/v3/<key>
export DEPLOYER_KEY=0x<hex>

forge create \
  --rpc-url "$ETH_RPC_URL" \
  --private-key "$DEPLOYER_KEY" \
  src/MockGpsVerifier.sol:MockGpsVerifier

# Or via cast (uses the compiled bytecode from out/)
BYTECODE=$(jq -r '.bytecode.object' out/MockGpsVerifier.sol/MockGpsVerifier.json)
cast send --rpc-url "$ETH_RPC_URL" --private-key "$DEPLOYER_KEY" \
  --create "$BYTECODE"
```

Capture the deployed address; set it as `MADARA_ORCHESTRATOR_MOCK_VERIFIER_ADDRESS` so
the orchestrator's mock prover registers facts there during the aggregator job.

## Manual inspection

```bash
# Check whether a fact is registered
cast call --rpc-url "$ETH_RPC_URL" <VERIFIER_ADDR> "isValid(bytes32)(bool)" <FACT_HASH>

# Register a fact manually (anyone can)
cast send --rpc-url "$ETH_RPC_URL" --private-key "$DEPLOYER_KEY" \
  <VERIFIER_ADDR> "registerFact(bytes32)" <FACT_HASH>
```

## Pointing the Starknet core contract at this verifier

The Starknet core contract reads its verifier address from storage (`VERIFIER_ADDRESS_TAG`).
During mocknet bootstrapping, deploy the core contract pointed at this mock's address, OR
use the admin `setVerifier(address)` flow if the core contract exposes one.

## Interface summary

```solidity
function registerFact(bytes32 fact) external;        // added by us; idempotent
function isValid(bytes32 fact) external view returns (bool);  // required by IFactRegistry
event FactRegistered(bytes32 indexed fact);
```

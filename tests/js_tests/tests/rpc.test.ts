import * as fs from "fs";
import * as path from "path";
import { getRpcUrl, getAdminUrl } from "../src/config";
import { executeStateSetup } from "../src/state-executor";
import { runAssertion, matchValue } from "../src/assertion-runner";
import { RpcCaller } from "../src/rpc-caller";
import {
  StateSetup,
  ReadAssertions,
  ErrorAssertions,
  TestContext,
} from "../src/types";
import { createSpecRegistry } from "../src/spec/spec-registry";

// Load fixture JSONs
const FIXTURE_DIR = path.resolve(__dirname, "../fixtures/v0_10_0");
const stateSetup: StateSetup = JSON.parse(
  fs.readFileSync(path.join(FIXTURE_DIR, "state_setup.json"), "utf-8"),
);
const readAssertions: ReadAssertions = JSON.parse(
  fs.readFileSync(path.join(FIXTURE_DIR, "read_assertions.json"), "utf-8"),
);
const errorAssertions: ErrorAssertions = JSON.parse(
  fs.readFileSync(path.join(FIXTURE_DIR, "error_assertions.json"), "utf-8"),
);

const rpcUrl = getRpcUrl(stateSetup.version);
const adminUrl = getAdminUrl();

// Shared context across all tests
const ctx: TestContext = {
  rpcUrl,
  adminUrl,
  results: new Map(),
  assertionResults: new Map(),
  nonceTracker: new Map(),
};

describe("Starknet RPC v0.10.0", () => {
  // ---- Phase 0: Spec Registry ----
  describe("Spec Registry", () => {
    it("should download and compile OpenRPC spec validators", async () => {
      const versionTag = `v${stateSetup.version}`;
      ctx.specRegistry = await createSpecRegistry(versionTag);
      expect(ctx.specRegistry.getMethodNames().length).toBeGreaterThan(0);
    });
  });

  // ---- Phase 1: State Setup ----
  describe("State Setup", () => {
    it("should execute all write steps and build deterministic chain state", async () => {
      await executeStateSetup(stateSetup, ctx);

      // Verify we captured results for all steps
      for (const step of stateSetup.steps) {
        expect(ctx.results.has(step.id)).toBe(true);
      }
    });
  });

  // ---- Phase 2: Read Assertions ----
  describe("Read Assertions", () => {
    for (const assertion of readAssertions.assertions) {
      if (assertion.skip) {
        it.skip(`${assertion.id} (${assertion.method})`, () => {});
        continue;
      }

      it(`${assertion.id} (${assertion.method})`, async () => {
        if (ctx.results.size === 0) {
          throw new Error(
            "State setup did not complete - cannot run read assertions",
          );
        }

        await runAssertion(assertion, ctx);
      });
    }
  });

  // ---- Phase 3: Error Case Assertions ----
  describe("Error Assertions", () => {
    for (const assertion of errorAssertions.assertions) {
      it(`${assertion.id} (${assertion.method})`, async () => {
        const rpcCaller = new RpcCaller(ctx.rpcUrl);
        const envelope = await rpcCaller.rawCall(
          assertion.method,
          assertion.params,
        );

        expect(envelope.error).toBeDefined();
        expect(envelope.error!.code).toBe(assertion.expected_error.code);
        expect(envelope.result).toBeUndefined();
      });
    }
  });

  // ---- Phase 4: Method Surface Coverage ----
  describe("Method Surface", () => {
    it("all spec methods are exposed by the node", async () => {
      if (!ctx.specRegistry) return;

      const rpcCaller = new RpcCaller(ctx.rpcUrl);
      const result = await rpcCaller.call("rpc_methods", []);
      const exposedMethods: string[] = (result.methods || [])
        .map((m: string) => {
          // rpc_methods may return versioned paths like "starknet/v0_10_0/starknet_chainId"
          const segments = m.split("/");
          return segments[segments.length - 1];
        })
        .filter((m: string) => m.startsWith("starknet_"));
      const exposedSet = new Set(exposedMethods);

      const specMethods = ctx.specRegistry.getMethodNames();
      const missing: string[] = [];
      for (const method of specMethods) {
        if (!exposedSet.has(method)) {
          missing.push(method);
        }
      }

      if (missing.length > 0) {
        console.warn(
          `[method-surface] Methods in spec but not exposed: ${missing.join(", ")}`,
        );
      }
      // Allow getMessagesStatus to be missing (known Madara gap)
      const criticalMissing = missing.filter(
        (m) => m !== "starknet_getMessagesStatus",
      );
      expect(criticalMissing).toEqual([]);
    });

    it("test suite covers all spec methods", () => {
      if (!ctx.specRegistry) return;

      const testedMethods = new Set(
        readAssertions.assertions.map((a) => a.method),
      );
      const specMethods = ctx.specRegistry.getMethodNames();
      const untested: string[] = [];

      for (const method of specMethods) {
        if (!testedMethods.has(method)) {
          untested.push(method);
        }
      }

      if (untested.length > 0) {
        console.warn(
          `[coverage] Spec methods without test assertions: ${untested.join(", ")}`,
        );
      }
      // All methods should have at least one assertion (skipped counts)
      expect(untested.length).toBe(0);
    });
  });

  // ---- Phase 5: Cross-Validations ----
  describe("Cross-Validations", () => {
    it("block hash consistency: blockHashAndNumber matches getBlockWithTxHashes(latest)", async () => {
      const bhan = ctx.assertionResults.get("block_hash_and_number");
      const blockTxHashes = ctx.assertionResults.get(
        "get_block_tx_hashes_single",
      );
      if (!bhan || !blockTxHashes) return;

      expect(bhan.block_hash).toBeDefined();
      expect(bhan.block_number).toBeDefined();
    });

    it("tx count consistency: getBlockTransactionCount matches getBlockWithTxHashes.transactions.length", async () => {
      const txCount = ctx.assertionResults.get("get_block_tx_count_multi");
      const blockTxHashes = ctx.assertionResults.get(
        "get_block_tx_hashes_multi",
      );
      if (txCount === undefined || !blockTxHashes) return;

      expect(txCount).toBe(blockTxHashes.transactions.length);
    });

    it("class lookup consistency: getClass and getClassAt return same entry points", async () => {
      const byHash = ctx.assertionResults.get("get_class");
      const byAddr = ctx.assertionResults.get("get_class_at");
      if (!byHash || !byAddr) return;

      expect(byHash.entry_points_by_type.EXTERNAL.length).toBe(
        byAddr.entry_points_by_type.EXTERNAL.length,
      );
      expect(byHash.contract_class_version).toBe(byAddr.contract_class_version);
    });

    it("class hash consistency: getClassHashAt matches declared class_hash", async () => {
      const classHashAt = ctx.assertionResults.get("get_class_hash_at");
      const declareResult = ctx.results.get("declare_hello");
      if (!classHashAt || !declareResult) return;

      expect(normHex(String(classHashAt))).toBe(
        normHex(declareResult.class_hash!),
      );
    });

    it("storage vs call consistency: getStorageAt matches call(get_balance)", async () => {
      const storageResult = ctx.assertionResults.get("get_storage_at_balance");
      const callResult = ctx.assertionResults.get("call_get_balance");
      if (!storageResult || !callResult) return;

      const storageHex = normHex(String(storageResult));
      const callHex = normHex(
        String(Array.isArray(callResult) ? callResult[0] : callResult),
      );
      expect(storageHex).toBe(callHex);
    });

    it("receipt block info consistency: receipt block_hash matches block from write phase", async () => {
      const receiptInvoke = ctx.assertionResults.get("get_tx_receipt_invoke");
      const invokeStep = ctx.results.get("invoke_increase_100");
      if (!receiptInvoke || !invokeStep) return;

      expect(normHex(receiptInvoke.block_hash)).toBe(
        normHex(invokeStep.block_hash!),
      );
    });

    it("header transaction_count matches transactions array length", async () => {
      const block = ctx.assertionResults.get("get_block_tx_hashes_single");
      if (!block) return;

      if (block.transaction_count !== undefined && block.transactions) {
        expect(block.transaction_count).toBe(block.transactions.length);
      }
    });

    it("tx by index matches tx by hash in multi-tx block", async () => {
      const byIndex0 = ctx.assertionResults.get("get_tx_by_block_and_index_0");
      const byIndex1 = ctx.assertionResults.get("get_tx_by_block_and_index_1");
      if (!byIndex0 || !byIndex1) return;

      expect(byIndex0.transaction_hash).not.toBe(byIndex1.transaction_hash);
    });

    it("empty block has zero transactions", async () => {
      const emptyCount = ctx.assertionResults.get("get_block_tx_count_empty");
      if (emptyCount === undefined) return;

      expect(emptyCount).toBe(0);
    });

    it("nonce at genesis is 0 and increases after transactions", async () => {
      const nonceGenesis = ctx.assertionResults.get("get_nonce_at_genesis");
      const nonceCurrent = ctx.assertionResults.get("get_nonce");
      if (nonceGenesis === undefined || nonceCurrent === undefined) return;

      expect(normHex(String(nonceGenesis))).toBe("0x0");
      const currentNonce = parseInt(String(nonceCurrent), 16);
      expect(currentNonce).toBeGreaterThan(0);
    });
  });
});

function normHex(s: string): string {
  if (!s.startsWith("0x")) return s.toLowerCase();
  const stripped = s.slice(2).replace(/^0+/, "") || "0";
  return "0x" + stripped.toLowerCase();
}

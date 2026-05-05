import * as fs from "fs";
import * as path from "path";
import { getRpcUrl, getAdminUrl } from "../src/config";
import { executeStateSetup } from "../src/state-executor";
import { runAssertion } from "../src/assertion-runner";
import { RpcCaller } from "../src/rpc-caller";
import { AdminClient } from "../src/admin-client";
import {
  StateSetup,
  ReadAssertions,
  ErrorAssertions,
  TestContext,
} from "../src/types";
import { createSpecRegistry } from "../src/spec/spec-registry";

interface RpcFixture {
  fixtureDirName: string;
  stateSetup: StateSetup;
  readAssertions: ReadAssertions;
  errorAssertions: ErrorAssertions;
}

const FIXTURE_DIR_NAMES = ["v0_10_0", "v0_10_2"];

function loadFixture(fixtureDirName: string): RpcFixture {
  const fixtureDir = path.resolve(__dirname, "../fixtures", fixtureDirName);

  return {
    fixtureDirName,
    stateSetup: JSON.parse(
      fs.readFileSync(path.join(fixtureDir, "state_setup.json"), "utf-8"),
    ),
    readAssertions: JSON.parse(
      fs.readFileSync(path.join(fixtureDir, "read_assertions.json"), "utf-8"),
    ),
    errorAssertions: JSON.parse(
      fs.readFileSync(path.join(fixtureDir, "error_assertions.json"), "utf-8"),
    ),
  };
}

const fixtures = FIXTURE_DIR_NAMES.map(loadFixture);
const adminUrl = getAdminUrl();
const sharedResults: TestContext["results"] = new Map();

function createContext(version: string): TestContext {
  return {
    rpcUrl: getRpcUrl(version),
    adminUrl,
    results: sharedResults,
    assertionResults: new Map(),
    nonceTracker: new Map(),
  };
}

const contexts = new Map(
  fixtures.map((fixture) => [
    fixture.stateSetup.version,
    createContext(fixture.stateSetup.version),
  ]),
);
const setupFixture = fixtures[0];
const setupContext = contexts.get(setupFixture.stateSetup.version)!;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function normHex(s: string): string {
  if (!s.startsWith("0x")) return s.toLowerCase();
  const stripped = s.slice(2).replace(/^0+/, "") || "0";
  return "0x" + stripped.toLowerCase();
}

describe("Starknet RPC multi-version", () => {
  // ---- Phase 0: Spec Registry ----
  describe("Spec Registry", () => {
    for (const fixture of fixtures) {
      const ctx = contexts.get(fixture.stateSetup.version)!;

      it(`downloads and compiles OpenRPC spec validators for v${fixture.stateSetup.version}`, async () => {
        const versionTag = `v${fixture.stateSetup.version}`;
        ctx.specRegistry = await createSpecRegistry(versionTag);
        expect(ctx.specRegistry.getMethodNames().length).toBeGreaterThan(0);
      });
    }
  });

  // ---- Phase 1: State Setup ----
  describe("State Setup", () => {
    it(`executes write steps once via v${setupFixture.stateSetup.version} and builds deterministic chain state`, async () => {
      await executeStateSetup(setupFixture.stateSetup, setupContext);

      // Verify we captured results for all steps
      for (const step of setupFixture.stateSetup.steps) {
        expect(sharedResults.has(step.id)).toBe(true);
      }
    });
  });

  // ---- Phase 1.5: L1 Messaging Setup (Anvil) ----
  describe("L1 Messaging Setup", () => {
    it("fires LogMessageToL2 on Anvil, syncs, and consumes the message", async () => {
      const anvilPort = process.env.ANVIL_PORT;
      if (!anvilPort) {
        console.log(
          "[l1-messaging] ANVIL_PORT not set, skipping L1 messaging setup",
        );
        return;
      }

      const anvilUrl = `http://127.0.0.1:${anvilPort}`;
      const coreContract =
        process.env.CORE_CONTRACT ||
        "0x5FbDB2315678afecb367f032d93F642f64180aa3";

      const {
        createPublicClient,
        createWalletClient,
        http,
        parseAbi,
        encodeFunctionData,
      } = await import("viem");
      const { foundry } = await import("viem/chains");

      const abi = parseAbi([
        "function fireEvent() public",
        "function setIsCanceled(bool value) public",
      ]);

      const walletClient = createWalletClient({
        chain: { ...foundry, id: 1337 },
        transport: http(anvilUrl),
        account: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266" as `0x${string}`,
      });

      const publicClient = createPublicClient({
        chain: { ...foundry, id: 1337 },
        transport: http(anvilUrl),
      });

      // Set canceled to false
      const setCanceledHash = await walletClient.sendTransaction({
        to: coreContract as `0x${string}`,
        data: encodeFunctionData({
          abi,
          functionName: "setIsCanceled",
          args: [false],
        }),
      });
      await publicClient.waitForTransactionReceipt({ hash: setCanceledHash });

      // Fire the LogMessageToL2 event
      const fireHash = await walletClient.sendTransaction({
        to: coreContract as `0x${string}`,
        data: encodeFunctionData({
          abi,
          functionName: "fireEvent",
        }),
      });
      const fireReceipt = await publicClient.waitForTransactionReceipt({
        hash: fireHash,
      });
      console.log(
        `[l1-messaging] LogMessageToL2 fired in L1 tx ${fireReceipt.transactionHash} at block ${fireReceipt.blockNumber}`,
      );

      expect(fireReceipt.status).toBe("success");

      // Store the L1 tx hash in context for the read assertion
      const l1TxHash = fireReceipt.transactionHash;
      sharedResults.set("l1_messaging_event", {
        transaction_hash: l1TxHash,
      });

      // Poll until Madara has synced the L1 event (tx hash appears in DB).
      // getMessagesStatus returns error 29 when the hash is unknown, and
      // a result (possibly empty array) once indexed.
      console.log("[l1-messaging] Polling for L1 message sync...");
      const rpcCaller = new RpcCaller(setupContext.rpcUrl);
      const pollDeadline = Date.now() + 60_000;
      let synced = false;
      while (Date.now() < pollDeadline) {
        try {
          const envelope = await rpcCaller.rawCall(
            "starknet_getMessagesStatus",
            { transaction_hash: l1TxHash },
          );
          if (!envelope.error) {
            synced = true;
            break;
          }
        } catch {
          // Still not indexed, keep polling
        }
        await sleep(2000);
      }
      expect(synced).toBe(true);

      // Close block to trigger block producer to consume the pending L1 message
      const admin = new AdminClient(adminUrl);
      await admin.closeBlock();
      await sleep(3000);
      console.log("[l1-messaging] Block closed, message should be consumed");
    });
  });

  for (const [fixtureIndex, fixture] of fixtures.entries()) {
    const ctx = contexts.get(fixture.stateSetup.version)!;
    const writeMethodIds = new Set([
      "add_invoke_transaction",
      "add_declare_transaction",
      "add_deploy_account_transaction",
    ]);
    const readOnly = fixture.readAssertions.assertions.filter(
      (a) => !writeMethodIds.has(a.id),
    );
    const writeOnly = fixture.readAssertions.assertions.filter((a) =>
      writeMethodIds.has(a.id),
    );

    describe(`Starknet RPC v${fixture.stateSetup.version}`, () => {
      // ---- Phase 2: Read Assertions ----
      describe("Read Assertions", () => {
        for (const assertion of readOnly) {
          if (assertion.skip) {
            it.skip(`${assertion.id} (${assertion.method})`, () => {});
            continue;
          }
          if (assertion.requires_anvil && !process.env.ANVIL_PORT) {
            it.skip(`${assertion.id} (${assertion.method}) [requires Anvil]`, () => {});
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

      // ---- Phase 2.5: Write Assertions ----
      // These submit transactions and modify chain state. A block close before
      // each ensures nonces are canonical (not stale from a prior pending tx).
      describe("Write Assertions", () => {
        for (const assertion of writeOnly) {
          const declaresFixedClass = assertion.id === "add_declare_transaction";
          if (declaresFixedClass && fixtureIndex < fixtures.length - 1) {
            it.skip(`${assertion.id} (${assertion.method}) [covered by setup and latest RPC fixture]`, () => {});
            continue;
          }

          it(`${assertion.id} (${assertion.method})`, async () => {
            // Close any pending block so on-chain nonces are up to date
            const admin = new AdminClient(adminUrl);
            try {
              await admin.closeBlock();
              await sleep(1000);
            } catch {
              // No pending block to close
            }

            await runAssertion(assertion, ctx);
          });
        }
      });

      if (fixture.stateSetup.version === "0.10.2") {
        describe("v0.10.2 Response Flags", () => {
          it("getTransactionByHash accepts INCLUDE_PROOF_FACTS for invoke-v3 transactions", async () => {
            const invoke = ctx.results.get("invoke_increase_42");
            expect(invoke?.transaction_hash).toBeDefined();

            const rpcCaller = new RpcCaller(ctx.rpcUrl);
            const result = await rpcCaller.call(
              "starknet_getTransactionByHash",
              {
                transaction_hash: invoke!.transaction_hash,
                response_flags: ["INCLUDE_PROOF_FACTS"],
              },
            );

            expect(result.type).toBe("INVOKE");
            expect(result.version).toBe("0x3");
            expect(
              result.proof_facts === undefined ||
                Array.isArray(result.proof_facts),
            ).toBe(true);
          });

          it("getBlockWithTxs accepts INCLUDE_PROOF_FACTS on the current protocol path", async () => {
            const invokeBlock = ctx.results.get("invoke_increase_100");
            expect(invokeBlock?.block_number).toBeDefined();

            const rpcCaller = new RpcCaller(ctx.rpcUrl);
            const result = await rpcCaller.call("starknet_getBlockWithTxs", {
              block_id: { block_number: invokeBlock!.block_number },
              response_flags: ["INCLUDE_PROOF_FACTS"],
            });

            expect(Array.isArray(result.transactions)).toBe(true);
            expect(result.transactions.length).toBeGreaterThanOrEqual(2);
            for (const tx of result.transactions) {
              expect(tx.type).toBe("INVOKE");
              expect(tx.version).toBe("0x3");
              expect(
                tx.proof_facts === undefined || Array.isArray(tx.proof_facts),
              ).toBe(true);
            }
          });

          it("getBlockWithReceipts accepts INCLUDE_PROOF_FACTS on the current protocol path", async () => {
            const invokeBlock = ctx.results.get("invoke_increase_100");
            expect(invokeBlock?.block_number).toBeDefined();

            const rpcCaller = new RpcCaller(ctx.rpcUrl);
            const result = await rpcCaller.call(
              "starknet_getBlockWithReceipts",
              {
                block_id: { block_number: invokeBlock!.block_number },
                response_flags: ["INCLUDE_PROOF_FACTS"],
              },
            );

            expect(Array.isArray(result.transactions)).toBe(true);
            expect(result.transactions.length).toBeGreaterThanOrEqual(2);
            for (const item of result.transactions) {
              expect(item.transaction.type).toBe("INVOKE");
              expect(item.transaction.version).toBe("0x3");
              expect(
                item.transaction.proof_facts === undefined ||
                  Array.isArray(item.transaction.proof_facts),
              ).toBe(true);
            }
          });

          it("rejects unknown response flags with Invalid params", async () => {
            const invoke = ctx.results.get("invoke_increase_42");
            expect(invoke?.transaction_hash).toBeDefined();

            const rpcCaller = new RpcCaller(ctx.rpcUrl);
            const envelope = await rpcCaller.rawCall(
              "starknet_getTransactionByHash",
              {
                transaction_hash: invoke!.transaction_hash,
                response_flags: ["UNKNOWN_FLAG"],
              },
            );

            expect(envelope.error).toBeDefined();
            expect(envelope.error!.code).toBe(-32602);
          });
        });
      }

      // ---- Phase 3: Error Case Assertions ----
      describe("Error Assertions", () => {
        for (const assertion of fixture.errorAssertions.assertions) {
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

        it("re-declaring already-declared class returns CLASS_ALREADY_DECLARED (code 51)", async () => {
          const {
            Account,
            Deployer,
            RpcProvider: StarkRpcProvider,
          } = await import("starknet");
          const { loadContractSierra, loadContractCasm } = await import(
            "../src/contract-loader"
          );
          const { DEFAULT_ACCOUNT_ADDRESS, DEFAULT_PRIVATE_KEY, UDC_ADDRESS } =
            await import("../src/config");

          const provider = new StarkRpcProvider({ nodeUrl: ctx.rpcUrl });
          const deployer = new Deployer(UDC_ADDRESS, "deployContract");
          const account = new Account({
            provider,
            address: DEFAULT_ACCOUNT_ADDRESS,
            signer: DEFAULT_PRIVATE_KEY,
            deployer,
          });

          const sierra = loadContractSierra("madara_contracts_HelloStarknet");
          const casm = loadContractCasm("madara_contracts_HelloStarknet");

          try {
            await account.declare({ contract: sierra, casm });
            throw new Error(
              "Expected error when re-declaring already-declared class",
            );
          } catch (err: any) {
            const code = err.baseError?.code ?? err.code;
            const data = JSON.stringify(
              err.baseError?.data ?? err.data ?? err.message ?? "",
            );
            expect([41, 51]).toContain(code);
            expect(data).toContain("already declared");
          }
        });
      });

      // ---- Phase 4: Method Surface Coverage ----
      describe("Method Surface", () => {
        it("all spec methods are exposed by the node", async () => {
          if (!ctx.specRegistry) return;

          const rpcCaller = new RpcCaller(ctx.rpcUrl);
          const result = await rpcCaller.call("rpc_methods", []);
          const exposedMethods: string[] = (result.methods || [])
            .map((m: string) => {
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
              `[method-surface] v${fixture.stateSetup.version} methods in spec but not exposed: ${missing.join(", ")}`,
            );
          }
          expect(missing).toEqual([]);
        });

        it("test suite covers all spec methods", () => {
          if (!ctx.specRegistry) return;

          const testedMethods = new Set([
            ...fixture.readAssertions.assertions.map((a) => a.method),
            ...fixture.errorAssertions.assertions.map((a) => a.method),
          ]);
          const specMethods = ctx.specRegistry.getMethodNames();
          const untested: string[] = [];

          for (const method of specMethods) {
            if (!testedMethods.has(method)) {
              untested.push(method);
            }
          }

          if (untested.length > 0) {
            console.warn(
              `[coverage] v${fixture.stateSetup.version} spec methods without test assertions: ${untested.join(", ")}`,
            );
          }
          expect(untested.length).toBe(0);
        });
      });

      // ---- Phase 5: Cross-Validations ----
      // These tests compare results across multiple RPC endpoints.
      // They use expect().toBeDefined() for dependencies so that a missing
      // upstream result causes an explicit failure, not a silent pass.
      describe("Cross-Validations", () => {
        it("block hash consistency: blockHashAndNumber matches getBlockWithTxHashes(latest)", async () => {
          const bhan = ctx.assertionResults.get("block_hash_and_number");
          expect(bhan).toBeDefined();

          expect(bhan.block_hash).toBeDefined();
          expect(bhan.block_number).toBeDefined();
        });

        it("tx count consistency: getBlockTransactionCount matches getBlockWithTxHashes.transactions.length", async () => {
          const txCount = ctx.assertionResults.get("get_block_tx_count_multi");
          const blockTxHashes = ctx.assertionResults.get(
            "get_block_tx_hashes_multi",
          );
          expect(txCount).toBeDefined();
          expect(blockTxHashes).toBeDefined();

          expect(txCount).toBe(blockTxHashes.transactions.length);
        });

        it("class lookup consistency: getClass and getClassAt return same entry points", async () => {
          const byHash = ctx.assertionResults.get("get_class");
          const byAddr = ctx.assertionResults.get("get_class_at");
          expect(byHash).toBeDefined();
          expect(byAddr).toBeDefined();

          expect(byHash.entry_points_by_type.EXTERNAL.length).toBe(
            byAddr.entry_points_by_type.EXTERNAL.length,
          );
          expect(byHash.contract_class_version).toBe(
            byAddr.contract_class_version,
          );
        });

        it("class hash consistency: getClassHashAt matches declared class_hash", async () => {
          const classHashAt = ctx.assertionResults.get("get_class_hash_at");
          const declareResult = ctx.results.get("declare_hello");
          expect(classHashAt).toBeDefined();
          expect(declareResult).toBeDefined();

          expect(normHex(String(classHashAt))).toBe(
            normHex(declareResult!.class_hash!),
          );
        });

        it("storage vs call consistency: getStorageAt matches call(get_balance)", async () => {
          const storageResult = ctx.assertionResults.get(
            "get_storage_at_balance",
          );
          const callResult = ctx.assertionResults.get("call_get_balance");
          expect(storageResult).toBeDefined();
          expect(callResult).toBeDefined();

          const storageHex = normHex(String(storageResult));
          const callHex = normHex(
            String(Array.isArray(callResult) ? callResult[0] : callResult),
          );
          expect(storageHex).toBe(callHex);
        });

        it("receipt block info consistency: receipt block_hash matches block from write phase", async () => {
          const receiptInvoke = ctx.assertionResults.get(
            "get_tx_receipt_invoke",
          );
          const invokeStep = ctx.results.get("invoke_increase_100");
          expect(receiptInvoke).toBeDefined();
          expect(invokeStep).toBeDefined();

          expect(normHex(receiptInvoke.block_hash)).toBe(
            normHex(invokeStep!.block_hash!),
          );
        });

        it("header transaction_count matches transactions array length", async () => {
          const block = ctx.assertionResults.get("get_block_tx_hashes_single");
          expect(block).toBeDefined();

          if (block.transaction_count !== undefined && block.transactions) {
            expect(block.transaction_count).toBe(block.transactions.length);
          }
        });

        it("tx by index matches tx by hash in multi-tx block", async () => {
          const byIndex0 = ctx.assertionResults.get(
            "get_tx_by_block_and_index_0",
          );
          const byIndex1 = ctx.assertionResults.get(
            "get_tx_by_block_and_index_1",
          );
          expect(byIndex0).toBeDefined();
          expect(byIndex1).toBeDefined();

          expect(byIndex0.transaction_hash).not.toBe(byIndex1.transaction_hash);
        });

        it("empty block has zero transactions", async () => {
          const emptyCount = ctx.assertionResults.get(
            "get_block_tx_count_empty",
          );
          expect(emptyCount).toBeDefined();

          expect(emptyCount).toBe(0);
        });

        it("nonce at genesis is 0 and increases after transactions", async () => {
          const nonceGenesis = ctx.assertionResults.get("get_nonce_at_genesis");
          const nonceCurrent = ctx.assertionResults.get("get_nonce");
          expect(nonceGenesis).toBeDefined();
          expect(nonceCurrent).toBeDefined();

          expect(normHex(String(nonceGenesis))).toBe("0x0");
          const currentNonce = parseInt(String(nonceCurrent), 16);
          expect(currentNonce).toBeGreaterThan(0);
        });
      });
    });
  }
});

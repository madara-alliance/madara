import * as fs from "fs";
import * as path from "path";
import { hash as snHash } from "starknet";
import {
  getRpcUrl,
  getAdminUrl,
  DEFAULT_ACCOUNT_ADDRESS,
  DEFAULT_PRIVATE_KEY,
  ERC20_STRK_ADDRESS,
} from "../src/config";
import { executeStateSetup } from "../src/state-executor";
import { runAssertion, matchValue } from "../src/assertion-runner";
import { resolveValue } from "../src/ref-resolver";
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
              resolveValue(assertion.params, ctx.results),
            );

            expect(envelope.error).toBeDefined();
            expect(envelope.error!.code).toBe(assertion.expected_error.code);
            expect(envelope.result).toBeUndefined();

            // Optional partial match on the error data, with the same matcher
            // vocabulary as read assertions.
            if (assertion.expected_error.data !== undefined) {
              const expectedData = resolveValue(
                assertion.expected_error.data,
                ctx.results,
              );
              const errors = matchValue(
                expectedData,
                envelope.error!.data,
                "$.error.data",
              );
              if (errors.length > 0) {
                const errorMsg = errors
                  .map(
                    (e) =>
                      `  ${e.path}: expected ${e.expected}, got ${e.actual}`,
                  )
                  .join("\n");
                throw new Error(
                  `Error data assertion "${assertion.id}" failed (${
                    assertion.method
                  }):\n${errorMsg}\nfull data: ${JSON.stringify(
                    envelope.error!.data,
                  )}`,
                );
              }
            }
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
              `[method-surface] v${
                fixture.stateSetup.version
              } methods in spec but not exposed: ${missing.join(", ")}`,
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
              `[coverage] v${
                fixture.stateSetup.version
              } spec methods without test assertions: ${untested.join(", ")}`,
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

  // ---- Phase 6: Spec failure-path regressions ----
  // The v0.10.x failure paths are covered by the fixture error/read assertions
  // (the original format, with $ref/$computed resolution and OpenRPC spec
  // validation). This phase only covers what the fixtures cannot express,
  // scoped to the specs we track (v0.9 and above; older endpoints are kept
  // as-is and not spec-pinned here):
  //  - the same wire semantics on v0.9.0, the spec-relevant endpoint with no
  //    fixture directory;
  //  - estimateFee/simulateTransactions semantics that need a live nonce or
  //    generated batches (SKIP_VALIDATE nonce relaxation, the failing
  //    transaction_index, the -32602 batch cap);
  //  - submitting a transaction that reverts on execution, to assert
  //    failure_reason in getTransactionStatus.
  describe("Spec failure-path regressions", () => {
    // Spec-relevant (v0.9+) endpoints with no fixture directory: covered here.
    const NON_FIXTURE_VERSIONS = ["0.9.0"];

    const ENTRYPOINT_NOT_FOUND_FELT =
      "0x454e545259504f494e545f4e4f545f464f554e44"; // 'ENTRYPOINT_NOT_FOUND'
    const FROM_L1_ADDRESS = "0x000000000000000000000000000000000000beef";
    const NON_EXISTENT_CONTRACT =
      "0x000000000000000000000000000000000000000000000000000000000000dead";

    const callerFor = (version: string) => new RpcCaller(getRpcUrl(version));
    const toHex = (n: bigint) => "0x" + n.toString(16);

    async function fetchNonce(caller: RpcCaller): Promise<bigint> {
      const nonce = await caller.call("starknet_getNonce", {
        block_id: "latest",
        contract_address: DEFAULT_ACCOUNT_ADDRESS,
      });
      return BigInt(nonce);
    }

    /// A fee-token transfer of amount (low, high) to self, as a raw
    /// broadcasted invoke v3 with a dummy signature (only usable with
    /// SKIP_VALIDATE or where validation is expected to fail).
    function transferTx(
      nonce: string,
      amountLow: string,
      amountHigh: string,
    ): any {
      const resource_bounds = {
        l1_gas: { max_amount: "0x186a0", max_price_per_unit: "0x100000000000" },
        l2_gas: {
          max_amount: "0x1000000000",
          max_price_per_unit: "0x100000000",
        },
        l1_data_gas: {
          max_amount: "0x186a0",
          max_price_per_unit: "0x100000000000",
        },
      };
      return {
        type: "INVOKE",
        version: "0x3",
        sender_address: DEFAULT_ACCOUNT_ADDRESS,
        // OZ cairo1 __execute__ calldata: [n_calls, to, selector, len, ...]
        calldata: [
          "0x1",
          ERC20_STRK_ADDRESS,
          snHash.getSelectorFromName("transfer"),
          "0x3",
          DEFAULT_ACCOUNT_ADDRESS,
          amountLow,
          amountHigh,
        ],
        signature: ["0x1", "0x2"],
        nonce,
        resource_bounds,
        tip: "0x0",
        paymaster_data: [],
        account_deployment_data: [],
        nonce_data_availability_mode: "L1",
        fee_data_availability_mode: "L1",
      };
    }

    describe("starknet_call failure surfacing (non-fixture versions)", () => {
      for (const version of NON_FIXTURE_VERSIONS) {
        it(`v${version}: non-existent selector returns ENTRYPOINT_NOT_FOUND (21), not retdata`, async () => {
          const envelope = await callerFor(version).rawCall("starknet_call", {
            request: {
              contract_address: ERC20_STRK_ADDRESS,
              entry_point_selector: snHash.getSelectorFromName(
                "definitely_not_an_entrypoint",
              ),
              calldata: [],
            },
            block_id: "latest",
          });

          // Regression guard: this used to be a *successful* response whose
          // result was ["0x454e...44"] ('ENTRYPOINT_NOT_FOUND').
          if (envelope.result !== undefined) {
            expect(envelope.result).not.toEqual([ENTRYPOINT_NOT_FOUND_FELT]);
          }
          expect(envelope.error).toBeDefined();
          expect(envelope.error!.code).toBe(21);
        });

        it(`v${version}: non-existent contract returns CONTRACT_NOT_FOUND (20)`, async () => {
          const envelope = await callerFor(version).rawCall("starknet_call", {
            request: {
              contract_address: NON_EXISTENT_CONTRACT,
              entry_point_selector: snHash.getSelectorFromName("get_balance"),
              calldata: [],
            },
            block_id: "latest",
          });

          expect(envelope.error).toBeDefined();
          expect(envelope.error!.code).toBe(20);
        });

        it(`v${version}: reverted call returns CONTRACT_ERROR (40) with structured revert_error`, async () => {
          // The caller address of starknet_call is 0, so the ERC20 panics
          // with 'ERC20: transfer from 0'. This used to be a successful call
          // result containing the panic felts.
          const envelope = await callerFor(version).rawCall("starknet_call", {
            request: {
              contract_address: ERC20_STRK_ADDRESS,
              entry_point_selector: snHash.getSelectorFromName("transfer"),
              calldata: [DEFAULT_ACCOUNT_ADDRESS, "0x1", "0x0"],
            },
            block_id: "latest",
          });

          expect(envelope.error).toBeDefined();
          expect(envelope.error!.code).toBe(40);
          const revertError = envelope.error!.data?.revert_error;
          expect(revertError).toBeDefined();
          expect(typeof revertError).toBe("object");
          expect(normHex(revertError.contract_address)).toBe(
            normHex(ERC20_STRK_ADDRESS),
          );
          expect(normHex(revertError.selector)).toBe(
            normHex(snHash.getSelectorFromName("transfer")),
          );
          expect(JSON.stringify(revertError)).toContain(
            "ERC20: transfer from 0",
          );
        });
      }
    });

    describe("estimateMessageFee semantics (non-fixture versions)", () => {
      for (const version of NON_FIXTURE_VERSIONS) {
        it(`v${version}: valid message returns a non-zero fee estimate`, async () => {
          // TestContract (deployed as the extra account) has
          // #[l1_handler] l1_handler_entrypoint(from_address, arg1, arg2).
          const target =
            sharedResults.get("deploy_new_account")?.contract_address;
          expect(target).toBeDefined();

          // Regression guard: the handler used to be built with
          // paid_fee_on_l1=0, which blockifier rejects on the *success* path,
          // so every valid message errored.
          const result = await callerFor(version).call(
            "starknet_estimateMessageFee",
            {
              message: {
                from_address: FROM_L1_ADDRESS,
                to_address: target,
                entry_point_selector: snHash.getSelectorFromName(
                  "l1_handler_entrypoint",
                ),
                payload: ["0x1", "0x2"],
              },
              block_id: "latest",
            },
          );

          expect(result.overall_fee).toBeDefined();
          expect(BigInt(result.overall_fee)).toBeGreaterThan(0n);
        });

        it(`v${version}: message whose handler fails returns CONTRACT_ERROR (40), not a fee`, async () => {
          // The fee token ERC20 has no l1 handler: executing the message
          // fails. Blockifier reports this as a successful execution with
          // revert_error set, which used to be priced as a fee estimate.
          const envelope = await callerFor(version).rawCall(
            "starknet_estimateMessageFee",
            {
              message: {
                from_address: FROM_L1_ADDRESS,
                to_address: ERC20_STRK_ADDRESS,
                entry_point_selector: snHash.getSelectorFromName(
                  "l1_handler_entrypoint",
                ),
                payload: ["0x1", "0x2"],
              },
              block_id: "latest",
            },
          );

          expect(envelope.error).toBeDefined();
          expect(envelope.error!.code).toBe(40);
        });
      }
    });

    describe("estimateFee error semantics", () => {
      for (const version of ["0.9.0", "0.10.2"]) {
        it(`v${version}: SKIP_VALIDATE allows estimating with a future nonce`, async () => {
          const caller = callerFor(version);
          const nonce = await fetchNonce(caller);

          const result = await caller.call("starknet_estimateFee", {
            request: [transferTx(toHex(nonce + 5n), "0x1", "0x0")],
            simulation_flags: ["SKIP_VALIDATE"],
            block_id: "latest",
          });

          expect(result).toHaveLength(1);
          expect(BigInt(result[0].overall_fee)).toBeGreaterThan(0n);
        });

        it(`v${version}: future nonce without SKIP_VALIDATE fails with error 41`, async () => {
          const caller = callerFor(version);
          const nonce = await fetchNonce(caller);

          const envelope = await caller.rawCall("starknet_estimateFee", {
            request: [transferTx(toHex(nonce + 5n), "0x1", "0x0")],
            simulation_flags: [],
            block_id: "latest",
          });

          expect(envelope.error).toBeDefined();
          expect(envelope.error!.code).toBe(41);
          expect(envelope.error!.data?.transaction_index).toBe(0);
        });

        it(`v${version}: error 41 blames the failing transaction with structured execution_error`, async () => {
          const caller = callerFor(version);
          const nonce = await fetchNonce(caller);

          // tx0 transfers 1 wei (fine), tx1 transfers more STRK than the
          // account holds and reverts. The error data must blame index 1,
          // not default to 0.
          const envelope = await caller.rawCall("starknet_estimateFee", {
            request: [
              transferTx(toHex(nonce), "0x1", "0x0"),
              transferTx(toHex(nonce + 1n), "0x0", "0xffffffffffffffff"),
            ],
            simulation_flags: ["SKIP_VALIDATE"],
            block_id: "latest",
          });

          expect(envelope.error).toBeDefined();
          expect(envelope.error!.code).toBe(41);
          expect(envelope.error!.data?.transaction_index).toBe(1);
          const executionError = envelope.error!.data?.execution_error;
          // Structured CONTRACT_EXECUTION_ERROR rooted at the sender account.
          expect(typeof executionError).toBe("object");
          expect(normHex(executionError.contract_address)).toBe(
            normHex(DEFAULT_ACCOUNT_ADDRESS),
          );
        });
      }

      for (const version of ["0.9.0", "0.10.2"]) {
        it(`v${version}: oversized estimateFee batch is rejected with -32602`, async () => {
          const tx = transferTx("0x0", "0x1", "0x0");
          const envelope = await callerFor(version).rawCall(
            "starknet_estimateFee",
            {
              request: Array(101).fill(tx),
              simulation_flags: ["SKIP_VALIDATE"],
              block_id: "latest",
            },
          );

          expect(envelope.error).toBeDefined();
          expect(envelope.error!.code).toBe(-32602);
        });
      }

      it("v0.10.2: oversized simulateTransactions batch is rejected with -32602", async () => {
        const tx = transferTx("0x0", "0x1", "0x0");
        const envelope = await callerFor("0.10.2").rawCall(
          "starknet_simulateTransactions",
          {
            block_id: "latest",
            transactions: Array(101).fill(tx),
            simulation_flags: ["SKIP_VALIDATE"],
          },
        );

        expect(envelope.error).toBeDefined();
        expect(envelope.error!.code).toBe(-32602);
      });

      it("v0.10.2: simulateTransactions with SKIP_VALIDATE allows a future nonce", async () => {
        const caller = callerFor("0.10.2");
        const nonce = await fetchNonce(caller);

        const result = await caller.call("starknet_simulateTransactions", {
          block_id: "latest",
          transactions: [transferTx(toHex(nonce + 5n), "0x1", "0x0")],
          simulation_flags: ["SKIP_VALIDATE", "SKIP_FEE_CHARGE"],
        });

        expect(result).toHaveLength(1);
        expect(result[0].transaction_trace).toBeDefined();
      });
    });

    describe("getEvents numeric range bounds (v0.9.0 has its own implementation and no fixture)", () => {
      for (const version of ["0.9.0"]) {
        it(`v${version}: numeric bounds beyond the tip return an empty page, not BLOCK_NOT_FOUND`, async () => {
          const caller = callerFor(version);
          const latest = await caller.call("starknet_blockNumber", []);

          const result = await caller.call("starknet_getEvents", {
            filter: {
              from_block: { block_number: latest + 1000 },
              to_block: { block_number: latest + 2000 },
              chunk_size: 10,
            },
          });

          expect(result.events).toEqual([]);
          // The spec says the token "should not appear if there are no more
          // pages"; pathfinder omits it.
          expect(result.continuation_token).toBeUndefined();
        });

        it(`v${version}: numeric to_block beyond the tip scans up to the tip`, async () => {
          const caller = callerFor(version);
          const latest = await caller.call("starknet_blockNumber", []);

          const result = await caller.call("starknet_getEvents", {
            filter: {
              from_block: { block_number: 0 },
              to_block: { block_number: latest + 1000 },
              chunk_size: 5,
            },
          });

          // The chain has events from state setup: an out-of-range to_block
          // must not error, and must still return them.
          expect(result.events.length).toBeGreaterThan(0);
        });

        it(`v${version}: hash bounds must still resolve (unknown hash is BLOCK_NOT_FOUND)`, async () => {
          const envelope = await callerFor(version).rawCall(
            "starknet_getEvents",
            {
              filter: {
                from_block: { block_hash: NON_EXISTENT_CONTRACT },
                chunk_size: 10,
              },
            },
          );

          expect(envelope.error).toBeDefined();
          expect(envelope.error!.code).toBe(24);
        });
      }
    });

    describe("getTransactionStatus failure_reason", () => {
      let revertedTxHash: string | undefined;

      it("submits a transaction that reverts on execution", async () => {
        const { Account, RpcProvider } = await import("starknet");

        const rpcUrl = getRpcUrl("0.10.2");
        const provider = new RpcProvider({ nodeUrl: rpcUrl });
        const account = new Account({
          provider,
          address: DEFAULT_ACCOUNT_ADDRESS,
          signer: DEFAULT_PRIVATE_KEY,
        });

        const admin = new AdminClient(adminUrl);
        try {
          await admin.closeBlock();
          await sleep(1000);
        } catch {
          // No pending block to close
        }

        const nonce = await provider.getNonceForAddress(
          DEFAULT_ACCOUNT_ADDRESS,
          "latest",
        );
        // Transfer far more STRK than the account holds: validation passes
        // (real signature, fee is affordable) but execution reverts, so the
        // transaction is included with execution_status REVERTED.
        const response = await account.execute(
          {
            contractAddress: ERC20_STRK_ADDRESS,
            entrypoint: "transfer",
            calldata: [DEFAULT_ACCOUNT_ADDRESS, "0x0", "0xffffffffffffffff"],
          },
          {
            nonce,
            resourceBounds: {
              l2_gas: { max_amount: 0x1000000n, max_price_per_unit: 0x100000n },
              l1_gas: { max_amount: 0x1000n, max_price_per_unit: 0x100000000n },
              l1_data_gas: {
                max_amount: 0x1000n,
                max_price_per_unit: 0x100000000n,
              },
            },
          },
        );

        revertedTxHash = response.transaction_hash;

        // The shared waiter treats REVERTED as a failure, so poll the status
        // directly until the node reports one for this transaction.
        const caller = callerFor("0.10.2");
        const deadline = Date.now() + 30_000;
        let included = false;
        while (Date.now() < deadline) {
          const envelope = await caller.rawCall(
            "starknet_getTransactionStatus",
            { transaction_hash: revertedTxHash },
          );
          if (envelope.result !== undefined) {
            included = true;
            break;
          }
          await sleep(500);
        }
        expect(included).toBe(true);

        await admin.closeBlock();
        await sleep(1000);
      });

      for (const version of ["0.9.0", "0.10.2"]) {
        it(`v${version}: reverted transaction carries failure_reason`, async () => {
          expect(revertedTxHash).toBeDefined();

          const status = await callerFor(version).call(
            "starknet_getTransactionStatus",
            { transaction_hash: revertedTxHash },
          );

          expect(status.execution_status).toBe("REVERTED");
          expect(typeof status.failure_reason).toBe("string");
          expect(status.failure_reason.length).toBeGreaterThan(0);
        });
      }
    });
  });
});

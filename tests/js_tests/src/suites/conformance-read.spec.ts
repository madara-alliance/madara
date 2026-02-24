import { ERC20_CONTRACT_ADDRESS } from "../fixtures/constants";
import { createRpcSession } from "../fixtures/rpc-session";
import { assertErrorEnvelope, assertSuccessEnvelope } from "../assertions/protocol-assertions";
import {
  assertParamsConformSpec,
  assertResultConformsSpec,
} from "../assertions/spec-assertions";
import { hex64, preferredPendingLikeTag, singleVersionFromEnv } from "./helpers";
import {
  type ReadConformanceState,
  prepareReadConformanceState,
} from "./conformance-state";

const version = singleVersionFromEnv();

interface ReadCaseContext {
  session: Awaited<ReturnType<typeof createRpcSession>>;
  state: ReadConformanceState;
}

interface ReadMethodCase {
  method: string;
  params: (ctx: ReadCaseContext) => unknown;
}

const READ_SUCCESS_CASES: ReadMethodCase[] = [
  {
    method: "starknet_specVersion",
    params: () => [],
  },
  {
    method: "starknet_getBlockWithTxHashes",
    params: ({ session }) => ({
      block_id: preferredPendingLikeTag(
        session.target.capabilities.supportsPreConfirmedTag,
      ),
    }),
  },
  {
    method: "starknet_getBlockWithTxs",
    params: ({ session }) => ({
      block_id: preferredPendingLikeTag(
        session.target.capabilities.supportsPreConfirmedTag,
      ),
    }),
  },
  {
    method: "starknet_getBlockWithReceipts",
    params: ({ session }) => ({
      block_id: preferredPendingLikeTag(
        session.target.capabilities.supportsPreConfirmedTag,
      ),
    }),
  },
  {
    method: "starknet_getStateUpdate",
    params: () => ({
      block_id: "latest",
    }),
  },
  {
    method: "starknet_getStorageAt",
    params: () => ({
      contract_address: ERC20_CONTRACT_ADDRESS,
      key: "0x0",
      block_id: "latest",
    }),
  },
  {
    method: "starknet_getTransactionStatus",
    params: ({ state }) => ({
      transaction_hash: state.transferTxHash,
    }),
  },
  {
    method: "starknet_getTransactionByHash",
    params: ({ state }) => ({
      transaction_hash: state.transferTxHash,
    }),
  },
  {
    method: "starknet_getTransactionByBlockIdAndIndex",
    params: ({ state }) => ({
      block_id: { block_number: state.touchedBlockNumber },
      index: state.touchedTransactionIndex,
    }),
  },
  {
    method: "starknet_getTransactionReceipt",
    params: ({ state }) => ({
      transaction_hash: state.transferTxHash,
    }),
  },
  {
    method: "starknet_getClass",
    params: ({ state }) => ({
      block_id: "latest",
      class_hash: state.erc20ClassHash,
    }),
  },
  {
    method: "starknet_getClassHashAt",
    params: () => ({
      block_id: "latest",
      contract_address: ERC20_CONTRACT_ADDRESS,
    }),
  },
  {
    method: "starknet_getClassAt",
    params: () => ({
      block_id: "latest",
      contract_address: ERC20_CONTRACT_ADDRESS,
    }),
  },
  {
    method: "starknet_getBlockTransactionCount",
    params: ({ state }) => ({
      block_id: { block_number: state.touchedBlockNumber },
    }),
  },
  {
    method: "starknet_call",
    params: ({ state }) => ({
      request: {
        contract_address: ERC20_CONTRACT_ADDRESS,
        entry_point_selector: state.erc20SelectorDecimals,
        calldata: [],
      },
      block_id: "latest",
    }),
  },
  {
    method: "starknet_estimateFee",
    params: () => ({
      request: [],
      simulation_flags: [],
      block_id: "latest",
    }),
  },
  {
    method: "starknet_blockNumber",
    params: () => [],
  },
  {
    method: "starknet_blockHashAndNumber",
    params: () => [],
  },
  {
    method: "starknet_chainId",
    params: () => [],
  },
  {
    method: "starknet_syncing",
    params: () => [],
  },
  {
    method: "starknet_getEvents",
    params: ({ state }) => ({
      filter: {
        from_block: { block_number: state.touchedBlockNumber },
        to_block: { block_number: state.touchedBlockNumber },
        chunk_size: 100,
        continuation_token: null,
      },
    }),
  },
  {
    method: "starknet_getNonce",
    params: ({ session }) => ({
      block_id: "latest",
      contract_address: session.fixtures.signer,
    }),
  },
  {
    method: "starknet_getStorageProof",
    params: () => ({
      block_id: "latest",
      class_hashes: [],
      contract_addresses: [],
      contracts_storage_keys: [],
    }),
  },
];

const READ_DOMAIN_CASES: ReadMethodCase[] = [
  {
    method: "starknet_getMessagesStatus",
    params: () => ({
      transaction_hash: hex64(1),
    }),
  },
  {
    method: "starknet_estimateMessageFee",
    params: ({ session }) => ({
      message: {
        from_address: "0x0000000000000000000000000000000000000001",
        to_address: session.fixtures.signer,
        entry_point_selector: "0x1",
        payload: ["0x1"],
      },
      block_id: "latest",
    }),
  },
];

describe(`Conformance Read Suite @ ${version}`, () => {
  let session: Awaited<ReturnType<typeof createRpcSession>>;
  let state: ReadConformanceState;
  let officialReadMethods: string[] = [];

  beforeAll(async () => {
    session = await createRpcSession(version);
    state = await prepareReadConformanceState(session);
    officialReadMethods = session.spec
      .listOfficialMethods()
      .filter((method) => session.spec.getMethodSource(method) === "read");
  });

  afterAll(async () => {
    await session.fixture.stop();
  });

  test("official read methods are exposed on rpc_methods", async () => {
    const methods = await session.rpcMethods();

    for (const method of officialReadMethods) {
      expect(methods.includes(method)).toBe(true);
    }
  });

  test("suite cases cover the full official read method surface", () => {
    const coveredMethods = new Set([
      ...READ_SUCCESS_CASES.map((testCase) => testCase.method),
      ...READ_DOMAIN_CASES.map((testCase) => testCase.method),
    ]);

    expect(coveredMethods.size).toBe(officialReadMethods.length);

    for (const method of officialReadMethods) {
      expect(coveredMethods.has(method)).toBe(true);
    }
  });

  test.each(READ_SUCCESS_CASES)(
    "conforms (success): $method",
    async ({ method, params: buildParams }) => {
      const params = buildParams({ session, state });
      assertParamsConformSpec(session.spec, method, params);

      const envelope = await session.transport.rawCall(method, params);
      assertSuccessEnvelope(envelope);
      assertResultConformsSpec(session.spec, method, envelope.result);
    },
  );

  test.each(READ_DOMAIN_CASES)(
    "conforms (domain outcome): $method",
    async ({ method, params: buildParams }) => {
      const params = buildParams({ session, state });
      assertParamsConformSpec(session.spec, method, params);

      const envelope = await session.transport.rawCall(method, params);
      if ("result" in envelope) {
        assertSuccessEnvelope(envelope);
        assertResultConformsSpec(session.spec, method, envelope.result);
        return;
      }

      assertErrorEnvelope(envelope);
      expect(envelope.error.code).not.toBe(-32601);
      expect(envelope.error.code).not.toBe(-32602);
    },
  );

  test("pending-family block tag semantics are reachable", async () => {
    const pendingLikeTag = preferredPendingLikeTag(
      session.target.capabilities.supportsPreConfirmedTag,
    );
    const params = { block_id: pendingLikeTag };
    assertParamsConformSpec(session.spec, "starknet_getBlockWithTxHashes", params);

    const envelope = await session.transport.rawCall(
      "starknet_getBlockWithTxHashes",
      params,
    );
    assertSuccessEnvelope(envelope);
    assertResultConformsSpec(
      session.spec,
      "starknet_getBlockWithTxHashes",
      envelope.result,
    );
  });

  test("unknown method returns JSON-RPC -32601", async () => {
    const envelope = await session.transport.rawCall("starknet_thisDoesNotExist", {});
    assertErrorEnvelope(envelope);
    expect(envelope.error.code).toBe(-32601);
  });

  test("known method with invalid params returns JSON-RPC -32602", async () => {
    const envelope = await session.transport.rawCall("starknet_getBlockWithTxs", {
      bad: "payload",
    });
    assertErrorEnvelope(envelope);
    expect(envelope.error.code).toBe(-32602);
  });
});

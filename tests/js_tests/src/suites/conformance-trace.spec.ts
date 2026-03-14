import { createRpcSession } from "../fixtures/rpc-session";
import { assertErrorEnvelope, assertSuccessEnvelope } from "../assertions/protocol-assertions";
import {
  assertParamsConformSpec,
  assertResultConformsSpec,
} from "../assertions/spec-assertions";
import { singleVersionFromEnv } from "./helpers";

const version = singleVersionFromEnv();

describe(`Conformance Trace Suite @ ${version}`, () => {
  let session: Awaited<ReturnType<typeof createRpcSession>>;

  beforeAll(async () => {
    session = await createRpcSession(version);
  });

  afterAll(async () => {
    await session.fixture.stop();
  });

  test("traceTransaction and traceBlockTransactions conform on a live tx", async () => {
    const tx = await session.txFlow.transferKnownToken(
      session.fixtures.receiver,
      1_000_002n,
    );
    const receipt = await session.txFlow.waitForTransaction(tx.txHash);

    const traceTxnParams = { transaction_hash: tx.txHash };
    assertParamsConformSpec(session.spec, "starknet_traceTransaction", traceTxnParams);

    const traceTxnEnvelope = await session.transport.rawCall(
      "starknet_traceTransaction",
      traceTxnParams,
    );
    assertSuccessEnvelope(traceTxnEnvelope);
    assertResultConformsSpec(
      session.spec,
      "starknet_traceTransaction",
      traceTxnEnvelope.result,
    );

    const blockNumber = (receipt as any).block_number;
    expect(typeof blockNumber).toBe("number");

    const traceBlockParams = { block_id: { block_number: blockNumber } };
    assertParamsConformSpec(
      session.spec,
      "starknet_traceBlockTransactions",
      traceBlockParams,
    );

    const traceBlockEnvelope = await session.transport.rawCall(
      "starknet_traceBlockTransactions",
      traceBlockParams,
    );
    assertSuccessEnvelope(traceBlockEnvelope);
    assertResultConformsSpec(
      session.spec,
      "starknet_traceBlockTransactions",
      traceBlockEnvelope.result,
    );
  });

  test("simulateTransactions conforms on empty transaction list", async () => {
    const params = {
      block_id: "latest",
      transactions: [],
      simulation_flags: [],
    };
    assertParamsConformSpec(session.spec, "starknet_simulateTransactions", params);

    const envelope = await session.transport.rawCall(
      "starknet_simulateTransactions",
      params,
    );
    assertSuccessEnvelope(envelope);
    assertResultConformsSpec(
      session.spec,
      "starknet_simulateTransactions",
      envelope.result,
    );
  });

  test("simulateTransactions malformed request returns JSON-RPC error", async () => {
    const envelope = await session.transport.rawCall("starknet_simulateTransactions", {
      bad: true,
    });

    assertErrorEnvelope(envelope);
    expect(envelope.error.code).not.toBe(-32601);
  });
});

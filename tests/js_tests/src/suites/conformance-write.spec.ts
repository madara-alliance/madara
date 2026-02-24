import { createRpcSession } from "../fixtures/rpc-session";
import { assertErrorEnvelope } from "../assertions/protocol-assertions";
import { assertResultConformsSpec } from "../assertions/spec-assertions";
import { singleVersionFromEnv } from "./helpers";

const version = singleVersionFromEnv();

describe(`Conformance Write Suite @ ${version}`, () => {
  let session: Awaited<ReturnType<typeof createRpcSession>>;

  beforeAll(async () => {
    session = await createRpcSession(version);
  });

  afterAll(async () => {
    await session.fixture.stop();
  });

  test("successful write flows conform to official write result schemas", async () => {
    const declared = await session.txFlow.declareContract(
      "madara_contracts_HelloStarknet",
    );
    assertResultConformsSpec(session.spec, "starknet_addDeclareTransaction", {
      transaction_hash: declared.txHash,
      class_hash: declared.classHash,
    });

    const invoked = await session.txFlow.transferKnownToken(
      session.fixtures.receiver,
      1_000_001n,
    );
    assertResultConformsSpec(session.spec, "starknet_addInvokeTransaction", {
      transaction_hash: invoked.txHash,
    });

    const deployedAccount = await session.txFlow.deployFreshAccount();
    assertResultConformsSpec(
      session.spec,
      "starknet_addDeployAccountTransaction",
      {
        transaction_hash: deployedAccount.txHash,
        contract_address: deployedAccount.contractAddress,
      },
    );
  });

  test("write methods reject malformed params with JSON-RPC error", async () => {
    const writeMethods = [
      "starknet_addInvokeTransaction",
      "starknet_addDeclareTransaction",
      "starknet_addDeployAccountTransaction",
    ];

    for (const method of writeMethods) {
      const envelope = await session.transport.rawCall(method, { invalid: true });
      assertErrorEnvelope(envelope);
      expect(envelope.error.code).not.toBe(-32601);
    }
  });

  test("write params validators are available from official spec", () => {
    expect(() =>
      session.spec.getMethodParamsValidator("starknet_addInvokeTransaction"),
    ).not.toThrow();
    expect(() =>
      session.spec.getMethodParamsValidator("starknet_addDeclareTransaction"),
    ).not.toThrow();
    expect(() =>
      session.spec.getMethodParamsValidator("starknet_addDeployAccountTransaction"),
    ).not.toThrow();
  });
});

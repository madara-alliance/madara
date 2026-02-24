import { createRpcSession } from "../fixtures/rpc-session";
import { assertErrorEnvelope, assertSuccessEnvelope } from "../assertions/protocol-assertions";
import { singleVersionFromEnv } from "./helpers";

const version = singleVersionFromEnv();

describe(`Madara Extension Suite @ ${version}`, () => {
  let session: Awaited<ReturnType<typeof createRpcSession>>;

  beforeAll(async () => {
    session = await createRpcSession(version);
  });

  afterAll(async () => {
    await session.fixture.stop();
  });

  test("non-spec methods are separated from strict official method set", async () => {
    const exposed = await session.rpcMethods();
    const official = new Set(session.spec.listOfficialMethods());

    const extras = exposed.filter((method) => !official.has(method));

    expect(extras.includes("starknet_getCompiledCasm")).toBe(true);

    for (const method of extras) {
      expect(session.target.capabilities.isOfficialSpecMethod(method)).toBe(false);
    }
  });

  test("getCompiledCasm extension is callable", async () => {
    const declared = await session.txFlow.declareContract(
      "madara_contracts_HelloStarknet",
    );

    const envelope = await session.transport.rawCall("starknet_getCompiledCasm", {
      class_hash: declared.classHash,
    });

    if ("error" in envelope) {
      assertErrorEnvelope(envelope);
      // method exists even if class lookup failed for a transient reason
      expect(envelope.error.code).not.toBe(-32601);
      return;
    }

    assertSuccessEnvelope(envelope);
    expect(envelope.result).toBeTruthy();
  });
});

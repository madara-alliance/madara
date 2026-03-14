import { withOfficialMethodMatcher } from "../config/capabilities";
import {
  buildRpcTarget,
  resolveRpcRootUrlFromEnv,
} from "../config/matrix";
import type { RpcSemver, RpcTarget } from "../config/types";
import { HttpJsonRpcTransport } from "../core/jsonrpc-transport";
import { SoftAssertCollector } from "../core/soft-assert";
import { TxE2EFlow } from "../flows/tx-e2e-flow";
import { createAccountFixture } from "./account-fixture";
import {
  DEFAULT_RECEIVER_ADDRESS,
  SIGNER_CONTRACT_ADDRESS,
} from "./constants";
import { MadaraFixture } from "./madara-fixture";
import { createSpecRegistry, type OpenRpcSpecRegistry } from "../spec/spec-registry";
import { ensureSpecCached } from "../spec/spec-cache";

export interface RpcSession {
  target: RpcTarget;
  fixture: MadaraFixture;
  spec: OpenRpcSpecRegistry;
  transport: HttpJsonRpcTransport;
  txFlow: TxE2EFlow;
  softAssert: SoftAssertCollector;
  fixtures: {
    receiver: string;
    signer: string;
  };
  rpcMethods: () => Promise<string[]>;
}

export async function createRpcSession(version: RpcSemver): Promise<RpcSession> {
  const rpcRootUrl = resolveRpcRootUrlFromEnv();
  const fixture = new MadaraFixture(rpcRootUrl);
  await fixture.start();

  const target = buildRpcTarget(version, rpcRootUrl);
  await ensureSpecCached(target.spec);
  const spec = createSpecRegistry(target.spec);
  const officialMethods = new Set(spec.listOfficialMethods());

  target.capabilities = withOfficialMethodMatcher(
    target.capabilities,
    officialMethods,
  );

  const accountFixture = createAccountFixture(target);
  const txFlow = new TxE2EFlow(accountFixture.provider, accountFixture.account, {
    supportsPreConfirmedTag: target.capabilities.supportsPreConfirmedTag,
  });
  const transport = new HttpJsonRpcTransport(target.baseUrl);
  const softAssert = new SoftAssertCollector();

  return {
    target,
    fixture,
    spec,
    transport,
    txFlow,
    softAssert,
    fixtures: {
      receiver: DEFAULT_RECEIVER_ADDRESS,
      signer: SIGNER_CONTRACT_ADDRESS,
    },
    rpcMethods: async () => {
      const result = await transport.call<{ methods: string[] }>("rpc_methods", []);
      const methods = result.methods || [];
      return Array.from(
        new Set(
          methods.map((entry) => {
            const segments = entry.split("/");
            return segments[segments.length - 1];
          }),
        ),
      )
        .filter((method) => method.startsWith("starknet_"))
        .sort();
    },
  };
}

import { buildRpcTarget } from "../config/matrix";
import type { RpcTarget, RpcSemver } from "../config/types";
import { waitFor } from "../core/poller";

export class MadaraFixture {
  constructor(public readonly rpcRootUrl: string) {}

  async start(): Promise<void> {
    await this.waitUntilReady();
  }

  async stop(): Promise<void> {
    // no-op: node lifecycle is handled by CI workflow
  }

  buildTarget(version: RpcSemver): RpcTarget {
    return buildRpcTarget(version, this.rpcRootUrl);
  }

  async waitUntilReady(): Promise<void> {
    const endpoint = this.rpcRootUrl.replace(/\/$/, "");

    await waitFor(
      async () => {
        const response = await fetch(endpoint, {
          method: "POST",
          headers: {
            "content-type": "application/json",
          },
          body: JSON.stringify({
            jsonrpc: "2.0",
            method: "starknet_chainId",
            params: [],
            id: 1,
          }),
        });

        const json = (await response.json()) as {
          result?: string;
          error?: { code: number; message: string };
        };
        return json;
      },
      (value) => Boolean(value.result) || Boolean(value.error),
      { timeoutMs: 90_000, intervalMs: 1000 },
    );
  }
}

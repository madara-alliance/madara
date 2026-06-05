import { BLOCK_POLL_INTERVAL_MS, BLOCK_POLL_TIMEOUT_MS } from "./config";
import { RpcCaller } from "./rpc-caller";

const INCLUDED_FINALITY_STATUSES = new Set([
  "PRE_CONFIRMED",
  "ACCEPTED_ON_L2",
  "ACCEPTED_ON_L1",
]);

export async function waitForPreConfirmedTransaction(
  rpcUrl: string,
  transactionHash: string,
): Promise<void> {
  const rpcCaller = new RpcCaller(rpcUrl);
  const deadline = Date.now() + BLOCK_POLL_TIMEOUT_MS;
  let lastObserved = "<none>";

  while (Date.now() < deadline) {
    const envelope = await rpcCaller.rawCall("starknet_getTransactionStatus", {
      transaction_hash: transactionHash,
    });

    if (envelope.error) {
      lastObserved = `error ${envelope.error.code}: ${envelope.error.message}`;
    } else {
      const status = envelope.result;
      lastObserved = JSON.stringify(status);

      if (status?.execution_status === "REVERTED") {
        throw new Error(`Transaction ${transactionHash} reverted`);
      }

      if (INCLUDED_FINALITY_STATUSES.has(status?.finality_status)) {
        return;
      }
    }

    await sleep(BLOCK_POLL_INTERVAL_MS);
  }

  throw new Error(
    `Timed out waiting for transaction ${transactionHash} to be pre-confirmed; last observed ${lastObserved}`,
  );
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

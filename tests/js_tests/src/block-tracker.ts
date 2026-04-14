import { RpcProvider } from "starknet";
import { AdminClient } from "./admin-client";
import { BLOCK_POLL_INTERVAL_MS, BLOCK_POLL_TIMEOUT_MS } from "./config";

export interface BlockInfo {
  block_number: number;
  block_hash: string;
}

/**
 * Manages deterministic block closure using the admin closeBlock() RPC.
 *
 * Since closeBlock() returns void, we poll blockNumber to detect
 * when the new block is available, then capture its hash.
 */
export class BlockTracker {
  private provider: RpcProvider;
  private admin: AdminClient;

  constructor(provider: RpcProvider, admin: AdminClient) {
    this.provider = provider;
    this.admin = admin;
  }

  async closeBlockAndTrack(): Promise<BlockInfo> {
    const prevBlockNumber = await this.provider.getBlockNumber();

    await this.admin.closeBlock();

    const newBlockNumber = await this.pollForBlockNumber(prevBlockNumber + 1);

    const block = await this.provider.getBlockLatestAccepted();

    if (block.block_number !== newBlockNumber) {
      throw new Error(
        `Block number mismatch after close: expected ${newBlockNumber}, got ${block.block_number}`,
      );
    }

    return {
      block_number: block.block_number,
      block_hash: block.block_hash,
    };
  }

  private async pollForBlockNumber(target: number): Promise<number> {
    const deadline = Date.now() + BLOCK_POLL_TIMEOUT_MS;

    while (Date.now() < deadline) {
      const current = await this.provider.getBlockNumber();
      if (current >= target) {
        return current;
      }
      await sleep(BLOCK_POLL_INTERVAL_MS);
    }

    throw new Error(
      `Timeout waiting for block ${target} after ${BLOCK_POLL_TIMEOUT_MS}ms`,
    );
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

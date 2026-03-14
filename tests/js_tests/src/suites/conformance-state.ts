import { hash } from "starknet";
import { ERC20_CONTRACT_ADDRESS } from "../fixtures/constants";
import type { RpcSession } from "../fixtures/rpc-session";

export interface ReadConformanceState {
  transferTxHash: string;
  transferReceipt: any;
  transferStatus: any;
  transferTxByHash: any;
  touchedBlockNumber: number;
  touchedTransactionIndex: number;
  erc20ClassHash: string;
  erc20SelectorDecimals: string;
}

export async function prepareReadConformanceState(
  session: RpcSession,
): Promise<ReadConformanceState> {
  const transfer = await session.txFlow.transferKnownToken(
    session.fixtures.receiver,
    1_000_000n,
  );

  const [transferTxByHash, transferReceipt, transferStatus] = await Promise.all([
    session.transport.call("starknet_getTransactionByHash", {
      transaction_hash: transfer.txHash,
    }),
    session.transport.call<any>("starknet_getTransactionReceipt", {
      transaction_hash: transfer.txHash,
    }),
    session.transport.call("starknet_getTransactionStatus", {
      transaction_hash: transfer.txHash,
    }),
  ]);

  const touchedBlockNumber =
    typeof transferReceipt.block_number === "number"
      ? transferReceipt.block_number
      : await session.transport.call<number>("starknet_blockNumber", []);

  const touchedTransactionIndex =
    typeof transferReceipt.transaction_index === "number"
      ? transferReceipt.transaction_index
      : 0;

  const erc20ClassHash = await session.transport.call<string>(
    "starknet_getClassHashAt",
    {
      block_id: "latest",
      contract_address: ERC20_CONTRACT_ADDRESS,
    },
  );

  return {
    transferTxHash: transfer.txHash,
    transferReceipt,
    transferStatus,
    transferTxByHash,
    touchedBlockNumber,
    touchedTransactionIndex,
    erc20ClassHash,
    erc20SelectorDecimals: hash.getSelectorFromName("decimals"),
  };
}

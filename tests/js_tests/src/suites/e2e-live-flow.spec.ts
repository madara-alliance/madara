import { createRpcSession } from "../fixtures/rpc-session";
import { assertSuccessEnvelope } from "../assertions/protocol-assertions";
import { assertBlockConsistency } from "../assertions/invariant-assertions";
import {
  assertParamsConformSpec,
  assertResultConformsSpec,
} from "../assertions/spec-assertions";
import { isLatestTarget } from "../config/capabilities";
import { singleVersionFromEnv, uniqueNumbers } from "./helpers";

const version = singleVersionFromEnv();

describe(`E2E Live Flow Suite @ ${version}`, () => {
  let session: Awaited<ReturnType<typeof createRpcSession>>;

  beforeAll(async () => {
    session = await createRpcSession(version);
  });

  afterAll(async () => {
    session.softAssert.flush("PLACEHOLDER_TODO");
    await session.fixture.stop();
  });

  test("couple txns -> fetch everything -> assert consistency", async () => {
    if (!isLatestTarget(session.target.semver)) {
      return;
    }

    const transferAmount = 100_000_000_000n;
    const receiverBalanceBefore = await session.txFlow.getReceiverBalance(
      session.fixtures.receiver,
    );
    const senderBalanceBefore = await session.txFlow.getSenderBalance();

    const declared = await session.txFlow.declareContract(
      "madara_contracts_HelloStarknet",
      { allowAlreadyDeclared: true },
    );
    const deployed = await session.txFlow.deployFromDeclared(declared.classHash);
    const transfer = await session.txFlow.transferKnownToken(
      session.fixtures.receiver,
      transferAmount,
    );

    const receiverBalanceAfter = await session.txFlow.getReceiverBalance(
      session.fixtures.receiver,
    );
    const senderBalanceAfter = await session.txFlow.getSenderBalance();

    expect(receiverBalanceAfter).toBe(receiverBalanceBefore + transferAmount);
    expect(senderBalanceAfter).toBeLessThanOrEqual(
      senderBalanceBefore - transferAmount,
    );

    const txHashes = [deployed.txHash, transfer.txHash];
    if (declared.txHash !== "0x0") {
      txHashes.unshift(declared.txHash);
    }

    const txByHash = await Promise.all(
      txHashes.map((hash) =>
        session.transport.call("starknet_getTransactionByHash", {
          transaction_hash: hash,
        }),
      ),
    );

    const receipts = await Promise.all(
      txHashes.map((hash) =>
        session.transport.call<any>("starknet_getTransactionReceipt", {
          transaction_hash: hash,
        }),
      ),
    );

    const statuses = await Promise.all(
      txHashes.map((hash) =>
        session.transport.call("starknet_getTransactionStatus", {
          transaction_hash: hash,
        }),
      ),
    );

    expect(txByHash).toHaveLength(txHashes.length);
    expect(receipts).toHaveLength(txHashes.length);
    expect(statuses).toHaveLength(txHashes.length);

    for (const hash of txHashes) {
      assertParamsConformSpec(session.spec, "starknet_getTransactionByHash", {
        transaction_hash: hash,
      });
      assertParamsConformSpec(session.spec, "starknet_getTransactionReceipt", {
        transaction_hash: hash,
      });
      assertParamsConformSpec(session.spec, "starknet_getTransactionStatus", {
        transaction_hash: hash,
      });
    }

    txByHash.forEach((result) =>
      assertResultConformsSpec(session.spec, "starknet_getTransactionByHash", result),
    );
    receipts.forEach((result) =>
      assertResultConformsSpec(
        session.spec,
        "starknet_getTransactionReceipt",
        result,
      ),
    );
    statuses.forEach((result) =>
      assertResultConformsSpec(
        session.spec,
        "starknet_getTransactionStatus",
        result,
      ),
    );

    for (const receipt of receipts) {
      if (
        typeof receipt?.block_number !== "number" ||
        typeof receipt?.transaction_index !== "number"
      ) {
        continue;
      }

      const envelope = await session.transport.rawCall(
        "starknet_getTransactionByBlockIdAndIndex",
        {
          block_id: { block_number: receipt.block_number },
          index: receipt.transaction_index,
        },
      );
      assertSuccessEnvelope(envelope);
      assertResultConformsSpec(
        session.spec,
        "starknet_getTransactionByBlockIdAndIndex",
        envelope.result,
      );
      expect((envelope.result as any).transaction_hash).toBe(
        receipt.transaction_hash,
      );
    }

    const touchedBlockNumbers = uniqueNumbers(
      receipts.map((receipt) => receipt?.block_number),
    );

    for (const blockNumber of touchedBlockNumbers) {
      const withHashesEnvelope = await session.transport.rawCall(
        "starknet_getBlockWithTxHashes",
        {
          block_id: { block_number: blockNumber },
        },
      );
      const withTxsEnvelope = await session.transport.rawCall("starknet_getBlockWithTxs", {
        block_id: { block_number: blockNumber },
      });
      const withReceiptsEnvelope = await session.transport.rawCall(
        "starknet_getBlockWithReceipts",
        {
          block_id: { block_number: blockNumber },
        },
      );
      const stateUpdateEnvelope = await session.transport.rawCall(
        "starknet_getStateUpdate",
        {
          block_id: { block_number: blockNumber },
        },
      );
      const txCountEnvelope = await session.transport.rawCall(
        "starknet_getBlockTransactionCount",
        {
          block_id: { block_number: blockNumber },
        },
      );

      assertSuccessEnvelope(withHashesEnvelope);
      assertSuccessEnvelope(withTxsEnvelope);
      assertSuccessEnvelope(withReceiptsEnvelope);
      assertSuccessEnvelope(stateUpdateEnvelope);
      assertSuccessEnvelope(txCountEnvelope);

      assertResultConformsSpec(
        session.spec,
        "starknet_getBlockWithTxHashes",
        withHashesEnvelope.result,
      );
      assertResultConformsSpec(
        session.spec,
        "starknet_getBlockWithTxs",
        withTxsEnvelope.result,
      );
      assertResultConformsSpec(
        session.spec,
        "starknet_getBlockWithReceipts",
        withReceiptsEnvelope.result,
      );
      assertResultConformsSpec(
        session.spec,
        "starknet_getStateUpdate",
        stateUpdateEnvelope.result,
      );
      assertResultConformsSpec(
        session.spec,
        "starknet_getBlockTransactionCount",
        txCountEnvelope.result,
      );

      assertBlockConsistency(
        withHashesEnvelope.result,
        withTxsEnvelope.result,
        withReceiptsEnvelope.result,
        stateUpdateEnvelope.result,
      );
    }

    const classHashAt = await session.txFlow.getClassHashAt(deployed.contractAddress);
    expect(classHashAt).toEqual(declared.classHash);

    const classDef = await session.txFlow.getClass(declared.classHash);
    expect(classDef).toBeTruthy();

    const classAt = await session.txFlow.getClassAt(deployed.contractAddress);
    expect(classAt).toBeTruthy();

    const classEnvelope = await session.transport.rawCall("starknet_getClass", {
      block_id: "latest",
      class_hash: declared.classHash,
    });
    assertSuccessEnvelope(classEnvelope);
    assertResultConformsSpec(session.spec, "starknet_getClass", classEnvelope.result);

    const classAtEnvelope = await session.transport.rawCall("starknet_getClassAt", {
      block_id: "latest",
      contract_address: deployed.contractAddress,
    });
    assertSuccessEnvelope(classAtEnvelope);
    assertResultConformsSpec(
      session.spec,
      "starknet_getClassAt",
      classAtEnvelope.result,
    );

    const eventsEnvelope = await session.transport.rawCall("starknet_getEvents", {
      filter: {
        from_block:
          touchedBlockNumbers.length > 0
            ? { block_number: touchedBlockNumbers[0] }
            : "latest",
        to_block:
          touchedBlockNumbers.length > 0
            ? {
                block_number:
                  touchedBlockNumbers[touchedBlockNumbers.length - 1],
              }
            : "latest",
        chunk_size: 100,
      },
    });

    assertSuccessEnvelope(eventsEnvelope);
    assertResultConformsSpec(session.spec, "starknet_getEvents", eventsEnvelope.result);

    const nonceEnvelope = await session.transport.rawCall("starknet_getNonce", {
      block_id: "latest",
      contract_address: session.fixtures.signer,
    });
    assertSuccessEnvelope(nonceEnvelope);
    assertResultConformsSpec(session.spec, "starknet_getNonce", nonceEnvelope.result);

    const storageProofEnvelope = await session.transport.rawCall(
      "starknet_getStorageProof",
      {
        block_id: "latest",
        class_hashes: [],
        contract_addresses: [deployed.contractAddress],
        contracts_storage_keys: [],
      },
    );
    assertSuccessEnvelope(storageProofEnvelope);
    assertResultConformsSpec(
      session.spec,
      "starknet_getStorageProof",
      storageProofEnvelope.result,
    );

    const estimateFee = await session.txFlow.estimateTransferFee(
      session.fixtures.receiver,
      10_000n,
    );

    session.softAssert.range("E2E_ESTIMATE_FEE_OVERALL", estimateFee.overall_fee, {
      min: 0n,
    });
    session.softAssert.todo(
      "E2E_ESTIMATE_FEE_EXACT_001",
      "exact expected estimate fee value pending real network baseline fixture",
    );
  });
});

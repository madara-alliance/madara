function txHashFromTx(tx: any): string | undefined {
  return tx?.transaction_hash;
}

export function assertBlockConsistency(
  withHashes: any,
  withTxs: any,
  withReceipts: any,
  stateUpdate: any,
): void {
  expect(withHashes.block_number).toBe(withTxs.block_number);
  expect(withHashes.block_number).toBe(withReceipts.block_number);
  expect(withHashes.block_hash).toBe(withTxs.block_hash);
  expect(withHashes.block_hash).toBe(withReceipts.block_hash);

  const hashesFromHashes = new Set((withHashes.transactions || []) as string[]);
  const hashesFromTxs = new Set(
    ((withTxs.transactions || []) as any[])
      .map((tx) => txHashFromTx(tx))
      .filter(Boolean),
  );
  const hashesFromReceipts = new Set(
    ((withReceipts.transactions || []) as any[])
      .map((entry) => entry?.receipt?.transaction_hash || entry?.transaction?.transaction_hash)
      .filter(Boolean),
  );

  expect(hashesFromHashes.size).toBe(hashesFromTxs.size);
  expect(hashesFromHashes.size).toBe(hashesFromReceipts.size);

  for (const hash of hashesFromHashes) {
    expect(hashesFromTxs.has(hash)).toBe(true);
    expect(hashesFromReceipts.has(hash)).toBe(true);
  }

  if (stateUpdate?.block_hash) {
    expect(stateUpdate.block_hash).toBe(withHashes.block_hash);
  }
}

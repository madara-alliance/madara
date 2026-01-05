module.exports = {
  async up(db) {
    // Migration: Convert tx_hashes array to tx_hash single value and set external_id
    //
    // This migration converts the tx_hashes field in StateTransition jobs
    // from an array of strings to a single optional string (tx_hash),
    // and also sets the external_id to the transaction hash.
    //
    // Before: { "metadata": { "specific": { "tx_hashes": ["0xabc..."] } }, "external_id": ... }
    // After:  { "metadata": { "specific": { "tx_hash": "0xabc..." } }, "external_id": "0xabc..." }
    //
    // Logic:
    // - If tx_hashes is empty or contains only empty/zero hashes -> tx_hash: null, external_id unchanged
    // - If tx_hashes has a valid hash -> tx_hash: last non-empty hash, external_id: same hash
    // - Remove the old tx_hashes field

    const jobsCollection = db.collection("jobs");

    // Find all StateTransition jobs that have tx_hashes field
    const cursor = jobsCollection.find({
      job_type: "StateTransition",
      "metadata.specific.tx_hashes": { $exists: true },
    });

    let updatedCount = 0;
    let errorCount = 0;

    const EMPTY_HASH = "0x" + "0".repeat(64);

    while (await cursor.hasNext()) {
      const doc = await cursor.next();

      try {
        const txHashes = doc.metadata?.specific?.tx_hashes || [];

        // Find the last non-empty hash (filter out empty/zero hashes)
        const validHashes = txHashes.filter(
          (hash) => hash && hash !== "" && hash !== EMPTY_HASH,
        );

        // Use the last valid hash, or null if none exist
        const txHash = validHashes.length > 0 ? validHashes[validHashes.length - 1] : null;

        // Build the update operation
        const updateOp = {
          $set: { "metadata.specific.tx_hash": txHash },
          $unset: { "metadata.specific.tx_hashes": "" },
        };

        // Also set external_id to the tx_hash if we have a valid hash
        if (txHash) {
          updateOp.$set.external_id = txHash;
        }

        await jobsCollection.updateOne({ _id: doc._id }, updateOp);

        updatedCount++;

        // Log progress every 1000 documents
        if (updatedCount % 1000 === 0) {
          console.log(`Migrated ${updatedCount} documents...`);
        }
      } catch (err) {
        console.error(`Failed to migrate document ${doc._id}: ${err.message}`);
        errorCount++;
      }
    }

    console.log(
      `Migration complete: ${updatedCount} documents updated, ${errorCount} errors`,
    );
  },

  async down(db) {
    // Rollback: Convert tx_hash back to tx_hashes array and reset external_id
    const jobsCollection = db.collection("jobs");

    // Find all StateTransition jobs that have tx_hash field
    const cursor = jobsCollection.find({
      job_type: "StateTransition",
      "metadata.specific.tx_hash": { $exists: true },
    });

    let updatedCount = 0;

    while (await cursor.hasNext()) {
      const doc = await cursor.next();

      const txHash = doc.metadata?.specific?.tx_hash;

      // Convert back to array format
      const txHashes = txHash ? [txHash] : [];

      await jobsCollection.updateOne(
        { _id: doc._id },
        {
          $set: {
            "metadata.specific.tx_hashes": txHashes,
            external_id: "", // Reset to empty string
          },
          $unset: { "metadata.specific.tx_hash": "" },
        },
      );

      updatedCount++;

      if (updatedCount % 1000 === 0) {
        console.log(`Rolled back ${updatedCount} documents...`);
      }
    }

    console.log(`Rollback complete: ${updatedCount} documents updated`);
  },
};

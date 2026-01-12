module.exports = {
  async up(db) {
    // Migration: Convert StateTransition job metadata from array format to single value format
    //
    // This migration converts the following fields in StateTransition jobs:
    // - tx_hashes: Vec<String> -> tx_hash: Option<String>
    // - snos_output_paths: Vec<String> -> snos_output_path: Option<String>
    // - program_output_paths: Vec<String> -> program_output_path: Option<String>
    // - blob_data_paths: Vec<String> -> blob_data_path: Option<String>
    // - da_segment_paths: Vec<String> -> da_segment_path: Option<String>
    // - context.to_settle: Vec<u64> -> context.to_settle: u64
    //
    // It also sets external_id to the transaction hash for tracking purposes.
    //
    // Logic for each array field:
    // - If array is empty or contains only empty values -> set to null
    // - If array has valid values -> use the last valid value (for tx_hashes) or first value (for paths)
    // - For context.to_settle: use the first value (since we now process one block/batch per job)
    // - Remove the old array field

    const jobsCollection = db.collection("jobs");

    // Find all StateTransition jobs
    const cursor = jobsCollection.find({
      job_type: "StateTransition",
    });

    let updatedCount = 0;
    let errorCount = 0;

    const EMPTY_HASH = "0x" + "0".repeat(64);

    while (await cursor.hasNext()) {
      const doc = await cursor.next();

      try {
        const specific = doc.metadata?.specific || {};
        const updateSet = {};
        const updateUnset = {};

        // Convert tx_hashes to tx_hash
        if (specific.tx_hashes !== undefined) {
          const txHashes = specific.tx_hashes || [];
          // Find the last non-empty hash (filter out empty/zero hashes)
          const validHashes = txHashes.filter(
            (hash) => hash && hash !== "" && hash !== EMPTY_HASH,
          );
          // Use the last valid hash, or null if none exist
          const txHash =
            validHashes.length > 0 ? validHashes[validHashes.length - 1] : null;

          updateSet["metadata.specific.tx_hash"] = txHash;
          updateUnset["metadata.specific.tx_hashes"] = "";

          // Also set external_id to the tx_hash if we have a valid hash
          if (txHash) {
            updateSet.external_id = txHash;
          }
        }

        // Convert snos_output_paths to snos_output_path
        if (specific.snos_output_paths !== undefined) {
          const paths = specific.snos_output_paths || [];
          const validPaths = paths.filter((p) => p && p !== "");
          updateSet["metadata.specific.snos_output_path"] =
            validPaths.length > 0 ? validPaths[0] : null;
          updateUnset["metadata.specific.snos_output_paths"] = "";
        }

        // Convert program_output_paths to program_output_path
        if (specific.program_output_paths !== undefined) {
          const paths = specific.program_output_paths || [];
          const validPaths = paths.filter((p) => p && p !== "");
          updateSet["metadata.specific.program_output_path"] =
            validPaths.length > 0 ? validPaths[0] : null;
          updateUnset["metadata.specific.program_output_paths"] = "";
        }

        // Convert blob_data_paths to blob_data_path
        if (specific.blob_data_paths !== undefined) {
          const paths = specific.blob_data_paths || [];
          const validPaths = paths.filter((p) => p && p !== "");
          updateSet["metadata.specific.blob_data_path"] =
            validPaths.length > 0 ? validPaths[0] : null;
          updateUnset["metadata.specific.blob_data_paths"] = "";
        }

        // Convert da_segment_paths to da_segment_path
        if (specific.da_segment_paths !== undefined) {
          const paths = specific.da_segment_paths || [];
          const validPaths = paths.filter((p) => p && p !== "");
          updateSet["metadata.specific.da_segment_path"] =
            validPaths.length > 0 ? validPaths[0] : null;
          updateUnset["metadata.specific.da_segment_paths"] = "";
        }

        // Convert context.to_settle from array to single value
        if (specific.context?.to_settle !== undefined) {
          const toSettle = specific.context.to_settle;
          if (Array.isArray(toSettle)) {
            // Use the first value from the array (or 0 if empty)
            updateSet["metadata.specific.context.to_settle"] =
              toSettle.length > 0 ? toSettle[0] : 0;
          }
          // If it's already a number, no change needed
        }

        // Only update if there are changes
        if (
          Object.keys(updateSet).length > 0 ||
          Object.keys(updateUnset).length > 0
        ) {
          const updateOp = {};
          if (Object.keys(updateSet).length > 0) {
            updateOp.$set = updateSet;
          }
          if (Object.keys(updateUnset).length > 0) {
            updateOp.$unset = updateUnset;
          }

          await jobsCollection.updateOne({ _id: doc._id }, updateOp);
          updatedCount++;

          // Log progress every 1000 documents
          if (updatedCount % 1000 === 0) {
            console.log(`Migrated ${updatedCount} documents...`);
          }
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
    // Rollback: Convert single value fields back to array format
    const jobsCollection = db.collection("jobs");

    // Find all StateTransition jobs
    const cursor = jobsCollection.find({
      job_type: "StateTransition",
    });

    let updatedCount = 0;

    while (await cursor.hasNext()) {
      const doc = await cursor.next();

      try {
        const specific = doc.metadata?.specific || {};
        const updateSet = {};
        const updateUnset = {};

        // Convert tx_hash back to tx_hashes
        if (specific.tx_hash !== undefined) {
          const txHash = specific.tx_hash;
          updateSet["metadata.specific.tx_hashes"] = txHash ? [txHash] : [];
          updateSet.external_id = ""; // Reset to empty string
          updateUnset["metadata.specific.tx_hash"] = "";
        }

        // Convert snos_output_path back to snos_output_paths
        if (specific.snos_output_path !== undefined) {
          const path = specific.snos_output_path;
          updateSet["metadata.specific.snos_output_paths"] = path ? [path] : [];
          updateUnset["metadata.specific.snos_output_path"] = "";
        }

        // Convert program_output_path back to program_output_paths
        if (specific.program_output_path !== undefined) {
          const path = specific.program_output_path;
          updateSet["metadata.specific.program_output_paths"] = path
            ? [path]
            : [];
          updateUnset["metadata.specific.program_output_path"] = "";
        }

        // Convert blob_data_path back to blob_data_paths
        if (specific.blob_data_path !== undefined) {
          const path = specific.blob_data_path;
          updateSet["metadata.specific.blob_data_paths"] = path ? [path] : [];
          updateUnset["metadata.specific.blob_data_path"] = "";
        }

        // Convert da_segment_path back to da_segment_paths
        if (specific.da_segment_path !== undefined) {
          const path = specific.da_segment_path;
          updateSet["metadata.specific.da_segment_paths"] = path ? [path] : [];
          updateUnset["metadata.specific.da_segment_path"] = "";
        }

        // Convert context.to_settle back to array
        if (specific.context?.to_settle !== undefined) {
          const toSettle = specific.context.to_settle;
          if (typeof toSettle === "number") {
            updateSet["metadata.specific.context.to_settle"] = [toSettle];
          }
          // If it's already an array, no change needed
        }

        // Only update if there are changes
        if (
          Object.keys(updateSet).length > 0 ||
          Object.keys(updateUnset).length > 0
        ) {
          const updateOp = {};
          if (Object.keys(updateSet).length > 0) {
            updateOp.$set = updateSet;
          }
          if (Object.keys(updateUnset).length > 0) {
            updateOp.$unset = updateUnset;
          }

          await jobsCollection.updateOne({ _id: doc._id }, updateOp);
          updatedCount++;

          if (updatedCount % 1000 === 0) {
            console.log(`Rolled back ${updatedCount} documents...`);
          }
        }
      } catch (err) {
        console.error(`Failed to rollback document ${doc._id}: ${err.message}`);
      }
    }

    console.log(`Rollback complete: ${updatedCount} documents updated`);
  },
};

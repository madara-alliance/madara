module.exports = {
  async up(db) {
    // Migration: Add orchestrator_version field to jobs and batches
    //
    // This migration adds the orchestrator_version field with value "0.1.0"
    // to all documents that don't already have this field.
    // Documents that already have the field will be left unchanged.
    //
    // Field paths:
    // - jobs: metadata.common.orchestrator_version
    // - snos_batches: orchestrator_version (root level)
    // - aggregator_batches: orchestrator_version (root level)

    const DEFAULT_VERSION = "0.1.0";

    // Update jobs collection - orchestrator_version is nested in metadata.common
    const jobsCollection = db.collection("jobs");
    const jobsResult = await jobsCollection.updateMany(
      { "metadata.common.orchestrator_version": { $exists: false } },
      { $set: { "metadata.common.orchestrator_version": DEFAULT_VERSION } },
    );
    console.log(
      `jobs: Updated ${jobsResult.modifiedCount} documents with metadata.common.orchestrator_version "${DEFAULT_VERSION}"`,
    );

    // Update batch collections - orchestrator_version is at root level
    const batchCollections = ["snos_batches", "aggregator_batches"];

    for (const collectionName of batchCollections) {
      const collection = db.collection(collectionName);

      const result = await collection.updateMany(
        { orchestrator_version: { $exists: false } },
        { $set: { orchestrator_version: DEFAULT_VERSION } },
      );

      console.log(
        `${collectionName}: Updated ${result.modifiedCount} documents with orchestrator_version "${DEFAULT_VERSION}"`,
      );
    }

    console.log(
      "Migration complete: orchestrator_version field added to all collections",
    );
  },

  async down(db) {
    // Rollback: Remove orchestrator_version field from all documents
    //
    // Note: This will remove the field from ALL documents, including those
    // that may have had a different version set after the migration.

    // Remove from jobs collection
    const jobsCollection = db.collection("jobs");
    const jobsResult = await jobsCollection.updateMany(
      { "metadata.common.orchestrator_version": { $exists: true } },
      { $unset: { "metadata.common.orchestrator_version": "" } },
    );
    console.log(
      `jobs: Removed metadata.common.orchestrator_version from ${jobsResult.modifiedCount} documents`,
    );

    // Remove from batch collections
    const batchCollections = ["snos_batches", "aggregator_batches"];

    for (const collectionName of batchCollections) {
      const collection = db.collection(collectionName);

      const result = await collection.updateMany(
        { orchestrator_version: { $exists: true } },
        { $unset: { orchestrator_version: "" } },
      );

      console.log(
        `${collectionName}: Removed orchestrator_version from ${result.modifiedCount} documents`,
      );
    }

    console.log(
      "Rollback complete: orchestrator_version field removed from all collections",
    );
  },
};

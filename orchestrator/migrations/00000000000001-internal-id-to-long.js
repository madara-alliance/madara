const { Long } = require("mongodb");

module.exports = {
  async up(db) {
    // Migration: Convert internal_id from String to Long (NumberLong)
    //
    // This migration converts the internal_id field in the jobs collection
    // from a string type to a 64-bit integer (Long) for better performance
    // and type consistency.
    //
    // Before: { "internal_id": "12345" }
    // After:  { "internal_id": NumberLong(12345) }

    const jobsCollection = db.collection("jobs");

    // Find all documents where internal_id is a string
    const cursor = jobsCollection.find({
      internal_id: { $type: "string" },
    });

    let updatedCount = 0;
    let errorCount = 0;

    while (await cursor.hasNext()) {
      const doc = await cursor.next();
      const internalIdStr = doc.internal_id;

      // Parse the string to a number
      const internalIdNum = parseInt(internalIdStr, 10);

      if (isNaN(internalIdNum)) {
        console.error(
          `Failed to parse internal_id "${internalIdStr}" for document ${doc._id}`,
        );
        errorCount++;
        continue;
      }

      // Update the document with the numeric value
      // Use Long() with string to avoid precision loss warnings
      await jobsCollection.updateOne(
        { _id: doc._id },
        { $set: { internal_id: Long.fromString(internalIdStr) } },
      );

      updatedCount++;

      // Log progress every 1000 documents
      if (updatedCount % 1000 === 0) {
        console.log(`Migrated ${updatedCount} documents...`);
      }
    }

    console.log(
      `Migration complete: ${updatedCount} documents updated, ${errorCount} errors`,
    );

    // Recreate the indexes that use internal_id to ensure proper ordering with the new type
    // First drop the existing indexes
    try {
      await jobsCollection.dropIndex("job_type_1_internal_id_-1");
    } catch (e) {
      console.log(
        "Index job_type_1_internal_id_-1 may not exist, continuing...",
      );
    }

    try {
      await jobsCollection.dropIndex("job_type_1_status_1_internal_id_-1");
    } catch (e) {
      console.log(
        "Index job_type_1_status_1_internal_id_-1 may not exist, continuing...",
      );
    }

    // Recreate the indexes
    await jobsCollection.createIndex(
      { job_type: 1, internal_id: -1 },
      { unique: true },
    );
    await jobsCollection.createIndex({
      job_type: 1,
      status: 1,
      internal_id: -1,
    });

    console.log("Indexes recreated successfully");
  },

  async down(db) {
    // Rollback: Convert internal_id from Long back to String
    const jobsCollection = db.collection("jobs");

    // Find all documents where internal_id is a number
    const cursor = jobsCollection.find({
      internal_id: { $type: "long" },
    });

    let updatedCount = 0;

    while (await cursor.hasNext()) {
      const doc = await cursor.next();
      const internalIdNum = doc.internal_id;

      // Convert back to string
      await jobsCollection.updateOne(
        { _id: doc._id },
        { $set: { internal_id: internalIdNum.toString() } },
      );

      updatedCount++;

      if (updatedCount % 1000 === 0) {
        console.log(`Rolled back ${updatedCount} documents...`);
      }
    }

    console.log(`Rollback complete: ${updatedCount} documents updated`);

    // Recreate the indexes
    try {
      await jobsCollection.dropIndex("job_type_1_internal_id_-1");
    } catch (e) {
      console.log(
        "Index job_type_1_internal_id_-1 may not exist, continuing...",
      );
    }

    try {
      await jobsCollection.dropIndex("job_type_1_status_1_internal_id_-1");
    } catch (e) {
      console.log(
        "Index job_type_1_status_1_internal_id_-1 may not exist, continuing...",
      );
    }

    await jobsCollection.createIndex(
      { job_type: 1, internal_id: -1 },
      { unique: true },
    );
    await jobsCollection.createIndex({
      job_type: 1,
      status: 1,
      internal_id: -1,
    });

    console.log("Indexes recreated successfully");
  },
};

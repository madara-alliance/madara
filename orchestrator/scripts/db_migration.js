const dotenv = require("dotenv").config();
const { MongoClient } = require("mongodb");
const fs = require("fs").promises;
const path = require("path");

// Configuration from environment variables
const MONGO_URL = process.env.MONGO_URL;
const COLLECTION_NAME = process.env.COLLECTION_NAME || "jobs";
const DATABASE_NAME = process.env.DATABASE_NAME || "jobs_db";
const BATCH_SIZE = process.env.BATCH_SIZE || 500;

// Progress tracking
const PROGRESS_FILE = "migration_progress.json";

class JobsMigration {
  constructor() {
    this.client = null;
    this.db = null;
    this.originalCollection = null;
    this.backupCollection = null;
    this.lastProcessedId = null;
    this.totalProcessed = 0;
    this.totalErrors = 0;
    this.startTime = null;
  }

  async connect() {
    if (!MONGO_URL) {
      throw new Error("MONGO_URL environment variable is required");
    }

    console.log("Connecting to MongoDB...");
    this.client = new MongoClient(MONGO_URL);
    await this.client.connect();
    this.db = this.client.db(DATABASE_NAME);
    this.originalCollection = this.db.collection(COLLECTION_NAME);
    this.backupCollection = this.db.collection(`${COLLECTION_NAME}_backup`);
    console.log("Connected successfully");
  }

  async disconnect() {
    if (this.client) {
      await this.client.close();
      console.log("Disconnected from MongoDB");
    }
  }

  async loadProgress() {
    try {
      const data = await fs.readFile(PROGRESS_FILE, "utf8");
      const progress = JSON.parse(data);
      this.lastProcessedId = progress.lastProcessedId;
      this.totalProcessed = progress.totalProcessed || 0;
      console.log(`Resuming from last processed ID: ${this.lastProcessedId}`);
      console.log(`Previously processed: ${this.totalProcessed} documents`);
    } catch (error) {
      console.log("No previous progress found, starting fresh migration");
      this.lastProcessedId = null;
      this.totalProcessed = 0;
    }
  }

  async saveProgress() {
    const progress = {
      lastProcessedId: this.lastProcessedId,
      totalProcessed: this.totalProcessed,
      totalErrors: this.totalErrors,
      timestamp: new Date().toISOString(),
    };
    await fs.writeFile(PROGRESS_FILE, JSON.stringify(progress, null, 2));
  }

  async createBackup() {
    console.log("Step 1: Creating backup collection...");

    // Check if backup already exists
    const collections = await this.db
      .listCollections({ name: `${COLLECTION_NAME}_backup` })
      .toArray();
    if (collections.length > 0) {
      console.log("Backup collection already exists, skipping backup creation");
      return;
    }

    // Create backup using aggregation pipeline
    const pipeline = [{ $match: {} }, { $out: `${COLLECTION_NAME}_backup` }];

    await this.originalCollection.aggregate(pipeline).toArray();

    // Verify backup
    const originalCount = await this.originalCollection.countDocuments();
    const backupCount = await this.backupCollection.countDocuments();

    console.log(`Original collection: ${originalCount} documents`);
    console.log(`Backup collection: ${backupCount} documents`);

    if (originalCount !== backupCount) {
      throw new Error(
        "Backup verification failed: document counts do not match",
      );
    }

    console.log("Backup created successfully ✓");
  }

  convertTimestamp(timestampStr) {
    if (!timestampStr) return null;

    // Convert from milliseconds string to seconds (Unix timestamp)
    const timestampMs = parseInt(timestampStr);
    if (isNaN(timestampMs)) return null;

    return Math.floor(timestampMs / 1000);
  }

  parseNumericValue(value, defaultValue = 0) {
    if (typeof value === "number") return value;
    if (typeof value === "string") {
      const parsed = parseInt(value);
      return isNaN(parsed) ? defaultValue : parsed;
    }
    return defaultValue;
  }

  transformMetadata(oldMetadata, jobType, internal_id) {
    const common = {
      process_attempt_no: this.parseNumericValue(
        oldMetadata.process_attempt_no,
      ),
      process_retry_attempt_no: this.parseNumericValue(
        oldMetadata.process_retry_attempt_no,
      ),
      verification_attempt_no: this.parseNumericValue(
        oldMetadata.verification_attempt_no,
      ),
      verification_retry_attempt_no: this.parseNumericValue(
        oldMetadata.verification_retry_attempt_no,
      ),
      process_started_at: this.convertTimestamp(
        oldMetadata.processing_started_at,
      ),
      process_completed_at: this.convertTimestamp(
        oldMetadata.processing_finished_at,
      ),
      verification_started_at: null, // New field, set to null
      verification_completed_at: null, // New field, set to null
      failure_reason: oldMetadata.failure_reason || oldMetadata.error || null,
    };

    let specific;

    let block_number = this.parseNumericValue(internal_id);
    switch (jobType) {
      case "SnosRun":
        specific = {
          type: "Snos",
          block_number: block_number,
          full_output: false, // Default value
          cairo_pie_path: `${block_number}/cairo_pie.zip`,
          snos_output_path: `${block_number}/snos_output.json`,
          program_output_path: `${block_number}/program_output.txt`,
          snos_fact: oldMetadata.snos_fact || null,
          snos_n_steps: null, // Will be populated later if exists
        };
        break;

      case "StateTransition":
        // Parse blocks to settle from comma-separated string
        const blocksStr = oldMetadata.blocks_number_to_settle || "";

        if (!blocksStr) {
          specific = {
            type: "StateUpdate",
            blocks_to_settle: [],
            snos_output_paths: [],
            program_output_paths: [],
            blob_data_paths: [],
            last_failed_block_no:
              this.parseNumericValue(oldMetadata.last_failed_block_no) || null,
            tx_hashes: this.extractTxHashes(oldMetadata),
          };
          break;
        }

        const blocks = blocksStr
          .split(",")
          .map((b) => this.parseNumericValue(b.trim()));

        const createPaths = (suffix) => blocks.map((blk) => `${blk}/${suffix}`);

        specific = {
          type: "StateUpdate",
          blocks_to_settle: blocks,
          snos_output_paths: createPaths("snos_output.json"),
          program_output_paths: createPaths("program_output.txt"),
          blob_data_paths: createPaths("blob_data.txt"),
          last_failed_block_no:
            this.parseNumericValue(oldMetadata.last_failed_block_no) || null,
          tx_hashes: this.extractTxHashes(oldMetadata),
        };
        break;

      case "DataSubmission":
        specific = {
          type: "Da",
          block_number: block_number,
          blob_data_path: `${block_number}/blob_data.txt`,
          tx_hash: oldMetadata.tx_hash || "NA",
        };
        break;

      case "ProofCreation":
        specific = {
          type: "Proving",
          block_number: block_number,
          input_path: {
            CairoPie: `${block_number}/cairo_pie.zip`,
          },
          ensure_on_chain_registration:
            oldMetadata.ensure_on_chain_registration || null,
          download_proof: `${block_number}/proof.json`,
          n_steps: this.parseNumericValue(oldMetadata.n_steps) || null,
        };
        break;

      case "ProofRegistration":
        // This might be similar to proving or have its own structure
        specific = {
          type: "Proving",
          block_number: block_number,
          input_path: {
            Proof: `${block_number}/proof.json`,
          },
          ensure_on_chain_registration:
            oldMetadata.ensure_on_chain_registration || null,
          download_proof: `${block_number}/proof_part2.json`,
          n_steps: null,
        };
        break;

      default:
        console.warn(`Unknown job type: ${jobType}, using default structure`);
        specific = {
          type: "Snos",
          block_number: 0,
          full_output: false,
          cairo_pie_path: null,
          snos_output_path: null,
          program_output_path: null,
          snos_fact: null,
          snos_n_steps: null,
        };
    }

    return {
      common,
      specific,
    };
  }

  extractTxHashes(oldMetadata) {
    const txHashes = [];

    // Look for attempt_tx_hashes_ prefixed keys
    for (const [key, value] of Object.entries(oldMetadata)) {
      if (key.startsWith("attempt_tx_hashes_") && value) {
        // Value might be comma-separated hashes
        const hashes = value
          .split(",")
          .map((h) => h.trim())
          .filter((h) => h.length > 0);
        txHashes.push(...hashes);
      }
    }

    return [...new Set(txHashes)]; // Remove duplicates
  }

  async processBatch() {
    console.log(
      `\nStep 2: Processing documents in batches of ${BATCH_SIZE}...`,
    );

    let query = {};
    if (this.lastProcessedId) {
      query._id = { $gt: this.lastProcessedId };
    }

    const cursor = this.originalCollection
      .find(query)
      .sort({ _id: 1 })
      .limit(parseInt(BATCH_SIZE));

    const batch = await cursor.toArray();

    if (batch.length === 0) {
      console.log("No more documents to process");
      return false;
    }

    console.log(`Processing batch of ${batch.length} documents...`);

    const bulkOps = [];
    let batchErrors = 0;

    for (const doc of batch) {
      try {
        const transformedMetadata = this.transformMetadata(
          doc.metadata || {},
          doc.job_type,
          doc.internal_id,
        );

        const updateDoc = {
          $set: {
            metadata: transformedMetadata,
            updated_at: new Date(),
          },
        };

        bulkOps.push({
          updateOne: {
            filter: { _id: doc._id },
            update: updateDoc,
          },
        });

        this.lastProcessedId = doc._id;
      } catch (error) {
        console.error(`Error processing document ${doc._id}:`, error.message);
        batchErrors++;
        this.totalErrors++;
      }
    }

    // Execute bulk operations
    if (bulkOps.length > 0) {
      try {
        const result = await this.originalCollection.bulkWrite(bulkOps, {
          ordered: false,
        });
        console.log(`Updated ${result.modifiedCount} documents`);
        this.totalProcessed += result.modifiedCount;
      } catch (error) {
        console.error("Bulk write error:", error.message);
        this.totalErrors += bulkOps.length;
      }
    }

    if (batchErrors > 0) {
      console.log(`Batch completed with ${batchErrors} errors`);
    }

    // Save progress after each batch
    await this.saveProgress();

    return true; // Continue processing
  }

  async migrate() {
    this.startTime = new Date();
    console.log(`\n🚀 Starting migration at ${this.startTime.toISOString()}`);
    console.log(`Database: ${DATABASE_NAME}`);
    console.log(`Collection: ${COLLECTION_NAME}`);
    console.log(`Batch size: ${BATCH_SIZE}`);

    try {
      await this.connect();
      await this.loadProgress();
      await this.createBackup();

      // Process all documents in batches
      let hasMore = true;
      while (hasMore) {
        hasMore = await this.processBatch();

        // Progress update
        const elapsed = (new Date() - this.startTime) / 1000;
        const rate = this.totalProcessed / elapsed;
        console.log(
          `Progress: ${this.totalProcessed} processed, ${this.totalErrors} errors, ${rate.toFixed(2)} docs/sec`,
        );
      }

      const endTime = new Date();
      const totalTime = (endTime - this.startTime) / 1000;

      console.log("\n✅ Migration completed successfully!");
      console.log(`Total time: ${totalTime.toFixed(2)} seconds`);
      console.log(`Documents processed: ${this.totalProcessed}`);
      console.log(`Errors: ${this.totalErrors}`);
      console.log(
        `Average rate: ${(this.totalProcessed / totalTime).toFixed(2)} docs/sec`,
      );

      // Clean up progress file on successful completion
      try {
        await fs.unlink(PROGRESS_FILE);
        console.log("Progress file cleaned up");
      } catch (error) {
        // Ignore error if file doesn't exist
      }
    } catch (error) {
      console.error("\n❌ Migration failed:", error.message);
      console.error(
        "Progress has been saved. You can resume the migration by running the script again.",
      );
      throw error;
    } finally {
      await this.disconnect();
    }
  }

  async validateMigration() {
    console.log("\n🔍 Running validation checks...");

    await this.connect();

    try {
      // Check if all documents have the new metadata structure
      const oldStructureCount = await this.originalCollection.countDocuments({
        "metadata.common": { $exists: false },
      });

      const newStructureCount = await this.originalCollection.countDocuments({
        "metadata.common": { $exists: true },
        "metadata.specific": { $exists: true },
      });

      const totalCount = await this.originalCollection.countDocuments();

      console.log(`Documents with old structure: ${oldStructureCount}`);
      console.log(`Documents with new structure: ${newStructureCount}`);
      console.log(`Total documents: ${totalCount}`);

      if (oldStructureCount === 0 && newStructureCount === totalCount) {
        console.log("✅ All documents have been successfully migrated");
      } else {
        console.log("⚠️  Some documents may not have been migrated correctly");
      }

      // Sample validation - check a few random documents
      const samples = await this.originalCollection
        .aggregate([
          { $match: { "metadata.common": { $exists: true } } },
          { $sample: { size: 3 } },
        ])
        .toArray();

      console.log("\nSample migrated documents:");
      samples.forEach((doc, index) => {
        console.log(`\nSample ${index + 1}:`);
        console.log(`Job Type: ${doc.job_type}`);
        console.log(`Metadata Structure: ${doc.metadata.specific.type}`);
        console.log(
          `Common fields: ${Object.keys(doc.metadata.common).length}`,
        );
        console.log(
          `Specific fields: ${Object.keys(doc.metadata.specific).length}`,
        );
      });
    } finally {
      await this.disconnect();
    }
  }
}

// Main execution
async function main() {
  const migration = new JobsMigration();

  const args = process.argv.slice(2);
  const command = args[0] || "migrate";

  try {
    switch (command) {
      case "migrate":
        await migration.migrate();
        break;
      case "validate":
        await migration.validateMigration();
        break;
      case "help":
        console.log(`
Usage: node migration-script.js [command]

Commands:
  migrate   - Run the migration (default)
  validate  - Validate the migration results
  help      - Show this help message

Environment Variables:
  MONGO_URL        - MongoDB connection string (required)
  DATABASE_NAME    - Database name (default: jobs_db)
  COLLECTION_NAME  - Collection name (default: jobs)
  BATCH_SIZE       - Batch size for processing (default: 500)

Example:
  MONGO_URL="mongodb+srv://admin:password@cluster0.fnbwa.mongodb.net/?retryWrites=true&w=majority&appName=Cluster0" \\
  DATABASE_NAME="production_db" \\
  COLLECTION_NAME="jobs" \\
  node migration-script.js migrate
                `);
        break;
      default:
        console.error(`Unknown command: ${command}`);
        process.exit(1);
    }
  } catch (error) {
    console.error("Script failed:", error.message);
    process.exit(1);
  }
}

// Handle graceful shutdown
process.on("SIGINT", async () => {
  console.log("\n⚠️  Received interrupt signal. Shutting down gracefully...");
  process.exit(0);
});

process.on("SIGTERM", async () => {
  console.log("\n⚠️  Received terminate signal. Shutting down gracefully...");
  process.exit(0);
});

if (require.main === module) {
  main();
}

module.exports = { JobsMigration };

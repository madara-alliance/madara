const { Long } = require("mongodb");

const LOOKUP_COLLECTION = "block_batch_lookups";
const SNOS_BATCHES_COLLECTION = "snos_batches";
const AGGREGATOR_BATCHES_COLLECTION = "aggregator_batches";
const RANGE_CHUNK_SIZE = 1000;

function toLong(value, fieldName) {
  if (value === null || value === undefined) {
    throw new Error(`Missing required numeric field: ${fieldName}`);
  }

  if (Long.isLong(value)) {
    return value;
  }

  if (typeof value === "number") {
    return Long.fromString(value.toString());
  }

  if (typeof value === "string") {
    return Long.fromString(value);
  }

  if (typeof value.toString === "function") {
    return Long.fromString(value.toString());
  }

  throw new Error(`Unsupported numeric field ${fieldName}: ${value}`);
}

function toSafeNumber(value, fieldName) {
  const parsed = Number.parseInt(toLong(value, fieldName).toString(), 10);

  if (!Number.isSafeInteger(parsed)) {
    throw new Error(`${fieldName} exceeds JavaScript safe integer range`);
  }

  return parsed;
}

async function rebuildLookupField(db, collectionName, targetField) {
  const sourceCollection = db.collection(collectionName);
  const lookupCollection = db.collection(LOOKUP_COLLECTION);
  const cursor = sourceCollection.find({});

  let processedDocuments = 0;
  let processedBlocks = 0;

  while (await cursor.hasNext()) {
    const document = await cursor.next();

    const startBlock = toSafeNumber(
      document.start_block,
      `${collectionName}.start_block`,
    );
    const endBlock = toSafeNumber(
      document.end_block,
      `${collectionName}.end_block`,
    );
    const batchIndex = toLong(document.index, `${collectionName}.index`);

    for (
      let chunkStart = startBlock;
      chunkStart <= endBlock;
      chunkStart += RANGE_CHUNK_SIZE
    ) {
      const chunkEnd = Math.min(chunkStart + RANGE_CHUNK_SIZE - 1, endBlock);
      const now = new Date();
      const operations = [];

      for (
        let blockNumber = chunkStart;
        blockNumber <= chunkEnd;
        blockNumber += 1
      ) {
        const blockNumberLong = Long.fromString(blockNumber.toString());

        operations.push({
          updateOne: {
            filter: { block_number: blockNumberLong },
            update: {
              $set: {
                [targetField]: batchIndex,
                updated_at: now,
              },
              $setOnInsert: {
                block_number: blockNumberLong,
                created_at: now,
              },
            },
            upsert: true,
          },
        });
      }

      if (operations.length > 0) {
        await lookupCollection.bulkWrite(operations, { ordered: false });
        processedBlocks += operations.length;
      }
    }

    processedDocuments += 1;

    if (processedDocuments % 100 === 0) {
      console.log(
        `${collectionName}: processed ${processedDocuments} source documents and ${processedBlocks} blocks`,
      );
    }
  }

  console.log(
    `${collectionName}: rebuilt ${targetField} for ${processedDocuments} documents covering ${processedBlocks} blocks`,
  );
}

async function dropLookupCollection(db) {
  try {
    await db.collection(LOOKUP_COLLECTION).drop();
  } catch (error) {
    if (error.codeName !== "NamespaceNotFound") {
      throw error;
    }
  }
}

module.exports = {
  async up(db) {
    await dropLookupCollection(db);

    const lookupCollection = db.collection(LOOKUP_COLLECTION);

    await lookupCollection.createIndex({ block_number: 1 }, { unique: true });
    await lookupCollection.createIndex({ snos_batch_index: 1 });
    await lookupCollection.createIndex({ aggregator_batch_index: 1 });

    await rebuildLookupField(
      db,
      AGGREGATOR_BATCHES_COLLECTION,
      "aggregator_batch_index",
    );
    await rebuildLookupField(db, SNOS_BATCHES_COLLECTION, "snos_batch_index");

    console.log("Migration complete: block batch lookups rebuilt");
  },

  async down(db) {
    await dropLookupCollection(db);
    console.log("Rollback complete: block batch lookups collection dropped");
  },
};

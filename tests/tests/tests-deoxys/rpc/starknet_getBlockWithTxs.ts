import axios from 'axios';
import * as dotenv from 'dotenv';
dotenv.config();

const REMOTE_RPC_URL = process.env.REMOTE_RPC;
const LOCAL_RPC_URL = process.env.LOCAL_RPC;
const START_BLOCK = 0;
const END_BLOCK = 100;
const MAX_CONCURRENT_REQUESTS = 10;

const requestDataForMethod = (method, params) => ({
  id: 1,
  jsonrpc: '2.0',
  method,
  params,
});

const compareObjects = (obj1: any, obj2: any, path: string = ""): string => {
  let differences = "";

  // Extract all unique keys from both objects
  const allKeys = new Set([...Object.keys(obj1), ...Object.keys(obj2)]);

  for (const key of allKeys) {
    const currentPath = path ? `${path}.${key}` : key;

    // Handle cases where a key is not present in one of the objects or is undefined
    if (obj1[key] === undefined) {
      differences += `\x1b[31mDIFFERENCE in Alchemy at ${currentPath}: ${obj2[key]}\x1b[0m\n`;
      continue;
    }

    if (obj2[key] === undefined) {
      differences += `\x1b[31mDIFFERENCE in Local at ${currentPath}: ${obj1[key]}\x1b[0m\n`;
      continue;
    }

    if (typeof obj1[key] === "object" && obj1[key] !== null) {
      differences += compareObjects(obj1[key], obj2[key], currentPath);
    } else if (obj1[key] !== obj2[key]) {
      differences += `\x1b[31mDIFFERENCE at ${currentPath}: ${obj1[key]} (Alchemy) vs ${obj2[key]} (Local)\x1b[0m\n`;
    } else {
      differences += `\x1b[32mMATCH at ${currentPath}: ${obj1[key]}\x1b[0m\n`;
    }
  }

  return differences;
};

async function benchmarkMethod(method, blockNumber) {
  console.log(`\x1b[34mBenchmarking method: ${method}\x1b[0m for block_number: ${blockNumber}`);

  const params = [{ block_number: blockNumber }];
  const alchemyResponse = await axios.post(REMOTE_RPC_URL, requestDataForMethod(method, params));
  const localResponse = await axios.post(LOCAL_RPC_URL, requestDataForMethod(method, params));

  const differences = compareObjects(alchemyResponse.data, localResponse.data);

  if (differences.includes('\x1b[31mDIFFERENCE')) {
    console.log(`\x1b[31mBlock ${blockNumber} has differences.\x1b[0m`);
  } else {
    console.log(`\x1b[32mBlock ${blockNumber} matches.\x1b[0m`);
  }

  return { blockNumber, differences };
}

async function checkDifferencesInBlocks() {
  let blocksWithDifferences = [];

  for (let blockNumber = START_BLOCK; blockNumber < END_BLOCK; blockNumber += MAX_CONCURRENT_REQUESTS) {
    const promises = [];
    for (let i = 0; i < MAX_CONCURRENT_REQUESTS && blockNumber + i < END_BLOCK; i++) {
      promises.push(benchmarkMethod('starknet_getBlockWithTxs', blockNumber + i));
    }

    const results = await Promise.all(promises);
    results.forEach(result => {
      if (result.differences.includes('\x1b[31mDIFFERENCE')) {
        blocksWithDifferences.push(result.blockNumber);
      }
    });
  }

  if (blocksWithDifferences.length === 0) {
    console.log('\x1b[32mAll blocks match!\x1b[0m');
  } else {
    console.log('\x1b[31mDifferences found in blocks:\x1b[0m', JSON.stringify(blocksWithDifferences));
  }
}

(async () => {
  // Loop through the blocks in batches
  await checkDifferencesInBlocks();
})();

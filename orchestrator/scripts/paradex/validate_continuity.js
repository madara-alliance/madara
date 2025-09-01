const fs = require("fs"); // Node.js built-in file system module
const path = require("path"); // Node.js built-in path module for handling file paths

/**
 * Checks the continuity of blocks in an array of objects.
 * Continuity is maintained if outputBlock[i] == inputBlock[i+1] for all valid indices.
 *
 * @param {Array<Object>} data - An array of block objects, each with 'inputBlock' and 'outputBlock' properties.
 * @returns {boolean} True if continuity is maintained, false otherwise.
 */
function checkBlockContinuity(data) {
  if (!Array.isArray(data) || data.length < 2) {
    console.log(
      "Input data is not an array or has insufficient length (less than 2 elements) for continuity check. Considering it continuous by default for this case.",
    );
    return true; // Or false, depending on your desired behavior for trivial cases
  }

  for (let i = 0; i < data.length - 1; i++) {
    const currentOutputBlock = data[i].outputBlock;
    const nextInputBlock = data[i + 1].inputBlock;

    if (currentOutputBlock === undefined || nextInputBlock === undefined) {
      console.warn(
        `Warning: Missing 'outputBlock' or 'inputBlock' at index ${i} or ${i + 1}. Cannot perform continuity check for these blocks.`,
      );
      continue; // Skip this pair if properties are missing
    }

    if (currentOutputBlock !== nextInputBlock) {
      console.error(`Continuity broken at index ${i}:`);
      console.error(`  data[${i}].outputBlock: ${currentOutputBlock}`);
      console.error(`  data[${i + 1}].inputBlock: ${nextInputBlock}`);
      return false; // Continuity broken
    }
  }

  console.log("Block continuity is maintained across all blocks.");
  return true; // All blocks are continuous
}

/**
 * Reads a JSON file from the given path, parses it, and then performs a continuity check.
 *
 * @param {string} filePath - The absolute or relative path to the JSON file.
 */
function processBlocksFromFile(filePath) {
  const absolutePath = path.resolve(filePath); // Resolve to an absolute path for robustness

  fs.readFile(absolutePath, "utf8", (err, data) => {
    if (err) {
      if (err.code === "ENOENT") {
        console.error(`Error: File not found at path: ${absolutePath}`);
      } else {
        console.error(`Error reading file from ${absolutePath}:`, err);
      }
      return;
    }

    try {
      const blockData = JSON.parse(data);
      console.log(`Successfully read data from: ${absolutePath}`);

      const isContinuous = checkBlockContinuity(blockData);

      if (isContinuous) {
        console.log("Overall result: All blocks are continuous!");
      } else {
        console.error(
          "Overall result: Continuity check failed for some blocks.",
        );
      }
    } catch (parseError) {
      console.error(`Error parsing JSON from ${absolutePath}:`, parseError);
    }
  });
}

// --- How to run this script ---
// You can provide the file path as a command-line argument,
// or hardcode it as shown below.

// Example 1: Hardcoded file path
const defaultFilePath = "./data/batches_simple.json";
console.log(`\n--- Processing default file: ${defaultFilePath} ---`);
processBlocksFromFile(defaultFilePath);

// Example 2: Using command-line argument
// To use this, run: node checkBlocks.js path/to/your/file.json
const commandLineFilePath = process.argv[2]; // process.argv[0] is 'node', process.argv[1] is the script name
if (commandLineFilePath) {
  console.log(
    `\n--- Processing file from command line: ${commandLineFilePath} ---`,
  );
  processBlocksFromFile(commandLineFilePath);
} else {
  console.log(
    `No file path provided as a command-line argument. Using default ${defaultFilePath}.`,
  );
  console.log(
    "To specify a file, run: node checkBlocks.js <your-file-path.json>",
  );
}

// Example 3: Test with broken data (uncomment to test)
/*
setTimeout(() => {
  console.log("\n--- Testing with intentionally broken data ---");
  const brokenDataPath = './broken_data.json';
  // Create a broken_data.json file for testing:
  // [ { "inputBlock": 1, "outputBlock": 2 }, { "inputBlock": 4, "outputBlock": 5 } ]
  fs.writeFileSync(brokenDataPath, JSON.stringify([
    { "inputBlock": 1, "outputBlock": 2 },
    { "inputBlock": 4, "outputBlock": 5 }, // Breaks continuity here
    { "inputBlock": 5, "outputBlock": 6 }
  ], null, 2));
  processBlocksFromFile(brokenDataPath);
}, 1000); // Small delay to separate output
*/

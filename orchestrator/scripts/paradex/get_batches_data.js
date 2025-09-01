const fetch = require("node-fetch"); // For Node.js environments
const fs = require("fs").promises; // For file operations

const BLOCK_OFFSET = 2 + 8 + 64 * 5;

/**
 * Fetches all transactions for a contract on Ethereum Sepolia with pagination
 * @param {string} contractAddress - The contract address to query
 * @param {string} apiKey - Your Etherscan API key (optional but recommended)
 * @param {number} startBlock - Starting block number
 * @param {number} endBlock - Ending block number
 * @param {string} outputFile - File path to write the results
 * @returns {Promise<Array>} Array of objects containing inputBlock and outputBlock
 */
async function getContractTransactions(
  contractAddress,
  apiKey = "",
  startBlock,
  endBlock,
  outputFile = "data/batches.json",
) {
  try {
    const baseUrl = "https://api-sepolia.etherscan.io/api";
    const offset = 1000; // Records per page
    let page = 1;
    let allBlocks = [];
    let hasMoreData = true;

    console.log(
      `Starting to fetch transactions from block ${startBlock} to ${endBlock}`,
    );
    console.log(`Using pagination with offset: ${offset}`);

    while (hasMoreData) {
      console.log(`Fetching page ${page}...`);

      // API parameters
      const params = new URLSearchParams({
        module: "account",
        action: "txlist",
        address: contractAddress,
        startblock: startBlock,
        endblock: endBlock,
        page: page,
        offset: offset,
        sort: "asc",
        apikey: apiKey,
      });

      const url = `${baseUrl}?${params}`;
      console.log(`Request URL: ${url}`);

      const response = await fetch(url);

      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }

      const data = await response.json();

      if (data.status !== "1") {
        if (data.message === "No transactions found") {
          console.log("No more transactions found");
          hasMoreData = false;
          break;
        } else {
          throw new Error(`API error: ${data.message}`);
        }
      }

      // console.log(data.result[1]);

      // Filter transactions with the specific method ID
      const filteredTransactions = data.result.filter(
        (tx) => tx.methodId === "0x507ee528",
      );
      // const filteredTransactions = data.result.filter(tx => tx.functionName.startsWith("updateState"));

      console.log(
        `Page ${page}: Found ${data.result.length} total transactions, ${filteredTransactions.length} with target method ID`,
      );

      if (filteredTransactions.length === 0) {
        console.log("No transactions with target method ID found on this page");
      } else {
        // Extract block ranges from transaction input data
        const blocks = filteredTransactions
          .map((tx, index) => {
            try {
              const input = tx.input;
              const startBlockHex = input.slice(
                BLOCK_OFFSET,
                BLOCK_OFFSET + 64,
              );
              const endBlockHex = input.slice(
                BLOCK_OFFSET + 64,
                BLOCK_OFFSET + 64 + 64,
              );

              const inputBlock = parseInt(startBlockHex, 16);
              const outputBlock = parseInt(endBlockHex, 16);

              return {
                txHash: tx.hash,
                blockNumber: tx.blockNumber,
                timeStamp: tx.timeStamp,
                inputBlock: inputBlock + 1,
                outputBlock,
              };
            } catch (parseError) {
              console.error(
                `Error parsing transaction ${tx.hash}:`,
                parseError.message,
              );
              return null;
            }
          })
          .filter((block) => block !== null); // Remove failed parses

        allBlocks.push(...blocks);
        console.log(
          `Extracted ${blocks.length} valid block ranges from page ${page}`,
        );
      }

      // Check if we have more data
      if (data.result.length < offset) {
        console.log("Reached end of data (less than offset returned)");
        hasMoreData = false;
      } else {
        page++;
        // Add a small delay to respect API rate limits
        await new Promise((resolve) => setTimeout(resolve, 200));
      }
    }

    console.log(`\nTotal transactions processed: ${allBlocks.length}`);

    // Write results to file
    if (allBlocks.length > 0) {
      const outputData = {
        metadata: {
          contractAddress,
          startBlock,
          endBlock,
          totalRecords: allBlocks.length,
          generatedAt: new Date().toISOString(),
        },
        data: allBlocks.map((block) => ({
          inputBlock: block.inputBlock,
          outputBlock: block.outputBlock,
          txHash: block.txHash,
          blockNumber: parseInt(block.blockNumber),
          timeStamp: block.timeStamp,
        })),
      };

      await fs.writeFile(outputFile, JSON.stringify(outputData, null, 2));
      console.log(`Results written to ${outputFile}`);

      // Also create a simple array version (just inputBlock and outputBlock)
      const simpleOutputFile = outputFile.replace(".json", "_simple.json");
      const simpleData = allBlocks.map((block) => ({
        inputBlock: block.inputBlock,
        outputBlock: block.outputBlock,
      }));

      await fs.writeFile(simpleOutputFile, JSON.stringify(simpleData, null, 2));
      console.log(`Simple format written to ${simpleOutputFile}`);
    } else {
      console.log("No data to write to file");
    }

    return allBlocks.map((block) => ({
      inputBlock: block.inputBlock,
      outputBlock: block.outputBlock,
    }));
  } catch (error) {
    console.error("Error fetching transactions:", error.message);
    throw error;
  }
}

/**
 * Alternative function that writes results in batches to handle very large datasets
 */
async function getContractTransactionsStreaming(
  contractAddress,
  apiKey = "",
  startBlock = 8827730,
  endBlock = 8836994,
  outputFile = "blockchain_data_streaming.json",
) {
  try {
    const baseUrl = "https://api-sepolia.etherscan.io/api";
    const offset = 500;
    let page = 1;
    let totalRecords = 0;
    let hasMoreData = true;

    // Initialize the output file with metadata and start the data array
    const metadata = {
      contractAddress,
      startBlock,
      endBlock,
      generatedAt: new Date().toISOString(),
    };

    await fs.writeFile(
      outputFile,
      `{"metadata":${JSON.stringify(metadata)},"data":[\n`,
    );

    console.log(`Starting streaming fetch to ${outputFile}`);

    while (hasMoreData) {
      console.log(`Fetching page ${page}...`);

      const params = new URLSearchParams({
        module: "account",
        action: "txlist",
        address: contractAddress,
        startblock: startBlock,
        endblock: endBlock,
        page: page,
        offset: offset,
        sort: "asc",
        apikey: apiKey,
      });

      const response = await fetch(`${baseUrl}?${params}`);
      const data = await response.json();

      if (data.status !== "1") {
        if (data.message === "No transactions found") {
          hasMoreData = false;
          break;
        } else {
          throw new Error(`API error: ${data.message}`);
        }
      }

      const filteredTransactions = data.result.filter(
        (tx) => tx.methodId === "0x507ee528",
      );

      if (filteredTransactions.length > 0) {
        const blocks = filteredTransactions.map((tx) => {
          const input = tx.input;
          const startBlockHex = input.slice(BLOCK_OFFSET, BLOCK_OFFSET + 64);
          const endBlockHex = input.slice(
            BLOCK_OFFSET + 64,
            BLOCK_OFFSET + 64 + 64,
          );

          return {
            inputBlock: parseInt(startBlockHex, 16),
            outputBlock: parseInt(endBlockHex, 16),
          };
        });

        // Append to file
        for (let i = 0; i < blocks.length; i++) {
          const comma = totalRecords > 0 ? "," : "";
          const blockData = JSON.stringify(blocks[i]);
          await fs.appendFile(outputFile, `${comma}\n  ${blockData}`);
          totalRecords++;
        }
      }

      if (data.result.length < offset) {
        hasMoreData = false;
      } else {
        page++;
        await new Promise((resolve) => setTimeout(resolve, 200));
      }
    }

    // Close the JSON structure
    await fs.appendFile(outputFile, `\n],"totalRecords":${totalRecords}}`);
    console.log(`Streaming complete. Total records: ${totalRecords}`);
  } catch (error) {
    console.error("Error in streaming fetch:", error.message);
    throw error;
  }
}

/**
 * Main function to demonstrate usage
 */
async function main() {
  const contractAddress = "0x11bACdFbBcd3Febe5e8CEAa75E0Ef6444d9B45FB";
  const apiKey = "HABZB4I5S1I24KWRUBNWFGURSXGHQVIBID";
  const startBlock = 6577430;
  const endBlock = 7986093;

  try {
    console.log("Fetching all transactions for contract:", contractAddress);
    console.log(`Block range: ${startBlock} to ${endBlock}`);

    const blocks = await getContractTransactions(
      contractAddress,
      apiKey,
      startBlock,
      endBlock,
      "data/batches.json",
    );

    console.log("\n=== Sample Results ===");
    blocks.slice(0, 5).forEach((block, index) => {
      console.log(`${index + 1}.`, block);
    });

    if (blocks.length > 5) {
      console.log(`... and ${blocks.length - 5} more records`);
    }

    return blocks;
  } catch (error) {
    console.error("Failed to fetch transactions:", error.message);
  }
}

// Export functions
module.exports = {
  getContractTransactions,
  getContractTransactionsStreaming,
};

// Run if called directly
if (require.main === module) {
  main()
    .then(() => console.log("Done!"))
    .catch((error) => console.error(error));
}

const fs = require('fs').promises;
const {MongoClient} = require('mongodb');
const {ethers} = require('ethers');

// Configuration from environment variables
const CONFIG = {
    mongoUrl: process.env.MONGO_URL || '',
    dbName: process.env.DB_NAME || '',
    collectionName: 'jobs',
    ethereumRpcUrl: process.env.RPC_URL || '',
    inputFilePath: process.env.INPUT_FILE_PATH || '',
    logFilePath: './blob_comparison_log.txt'
};

class BlobCommitmentMatcher {
    constructor(maxIndexes = null) {
        this.mongoClient = null;
        this.ethProvider = null;
        this.logEntries = [];
        this.maxIndexes = maxIndexes;
    }

    async initialize() {
        // Validate required environment variables
        const requiredEnvVars = {
            'MONGO_URL': CONFIG.mongoUrl,
            'DB_NAME': CONFIG.dbName,
            'RPC_URL': CONFIG.ethereumRpcUrl,
            'INPUT_FILE_PATH': CONFIG.inputFilePath
        };

        for (const [varName, value] of Object.entries(requiredEnvVars)) {
            if (!value) {
                throw new Error(`Required environment variable ${varName} is not set`);
            }
        }

        // Initialize MongoDB connection
        this.mongoClient = new MongoClient(CONFIG.mongoUrl);
        await this.mongoClient.connect();
        console.log('Connected to MongoDB');

        // Initialize Ethereum provider
        this.ethProvider = new ethers.JsonRpcProvider(CONFIG.ethereumRpcUrl);
        console.log('Connected to Ethereum RPC');
    }

    async readInputData() {
        try {
            const data = await fs.readFile(CONFIG.inputFilePath, 'utf8');
            const parsedData = JSON.parse(data);

            // Limit the data if maxIndexes is specified
            if (this.maxIndexes && this.maxIndexes > 0) {
                return parsedData.slice(0, this.maxIndexes);
            }

            return parsedData;
        } catch (error) {
            throw new Error(`Failed to read input file: ${error.message}`);
        }
    }

    async findMongoEntry(index) {
        const db = this.mongoClient.db(CONFIG.dbName);
        const collection = db.collection(CONFIG.collectionName);

        const query = {job_type: "StateTransition", internal_id: index.toString()};
        const result = await collection.findOne(query);

        if (!result) {
            throw new Error(`No MongoDB entry found with internal_id: ${index}`);
        }

        return result;
    }

    async getBlobCommitments(txHash) {
        try {
            // Get transaction receipt to find blob-related data
            const receipt = await this.ethProvider.getTransactionReceipt(txHash);
            if (!receipt) {
                throw new Error(`Transaction receipt not found for hash: ${txHash}`);
            }

            // Get the full transaction details
            const tx = await this.ethProvider.getTransaction(txHash);
            if (!tx) {
                throw new Error(`Transaction not found for hash: ${txHash}`);
            }

            // Extract blob commitments from transaction
            // Note: This depends on the transaction type and how blobs are stored
            // For EIP-4844 blob transactions, commitments are in blobVersionedHashes
            let blobCommitments = [];

            if (tx.blobVersionedHashes && tx.blobVersionedHashes.length > 0) {
                blobCommitments = tx.blobVersionedHashes;
            } else {
                // If no direct blob hashes, try to extract from logs or calldata
                // This is a fallback method - you may need to adapt based on your specific use case
                console.warn(`No blob versioned hashes found for tx: ${txHash}`);

                // Look for blob-related events in logs
                if (receipt.logs && receipt.logs.length > 0) {
                    for (const log of receipt.logs) {
                        // Check if log contains blob commitment data
                        // This is implementation-specific based on your contract's events
                        if (log.topics && log.topics.length > 0) {
                            // Extract potential blob commitments from log topics/data
                            // You'll need to adapt this based on your specific log structure
                            blobCommitments.push(...this.extractBlobCommitmentsFromLog(log));
                        }
                    }
                }
            }

            return blobCommitments;
        } catch (error) {
            throw new Error(`Failed to get blob commitments for ${txHash}: ${error.message}`);
        }
    }

    extractBlobCommitmentsFromLog(log) {
        // This is a placeholder method - implement based on your specific log structure
        // Example: if blob commitments are stored in log data or topics
        const commitments = [];

        // Example implementation:
        // if (log.topics[0] === 'YOUR_BLOB_COMMITMENT_EVENT_SIGNATURE') {
        //     commitments.push(log.data);
        // }

        return commitments;
    }

    compareBlobCommitments(commitments1, commitments2, txHash1, txHash2, index) {
        const logEntry = {
            index,
            txHash1,
            txHash2,
            timestamp: new Date().toISOString(),
            status: 'SUCCESS',
            message: '',
            blobCount1: commitments1.length,
            blobCount2: commitments2.length,
            commitmentsMatch: false
        };

        try {
            // Check if blob counts match
            if (commitments1.length !== commitments2.length) {
                logEntry.status = 'ERROR';
                logEntry.message = `Blob count mismatch: ${commitments1.length} vs ${commitments2.length}`;
                return logEntry;
            }

            // Sort commitments for comparison
            const sorted1 = [...commitments1].sort();
            const sorted2 = [...commitments2].sort();

            // Compare each commitment
            let allMatch = true;
            for (let i = 0; i < sorted1.length; i++) {
                if (sorted1[i] !== sorted2[i]) {
                    allMatch = false;
                    break;
                }
            }

            logEntry.commitmentsMatch = allMatch;

            if (allMatch) {
                logEntry.message = `All ${commitments1.length} blob commitments match`;
            } else {
                logEntry.status = 'ERROR';
                logEntry.message = 'Blob commitments do not match';
            }

        } catch (error) {
            logEntry.status = 'ERROR';
            logEntry.message = `Comparison failed: ${error.message}`;
        }

        return logEntry;
    }

    async writeLogEntry(logEntry) {
        const logLine = `[${logEntry.timestamp}] Index: ${logEntry.index} | Status: ${logEntry.status} | ${logEntry.message} | TxHash1: ${logEntry.txHash1} | TxHash2: ${logEntry.txHash2} | BlobCounts: ${logEntry.blobCount1}/${logEntry.blobCount2}\n`;

        try {
            await fs.appendFile(CONFIG.logFilePath, logLine);
            console.log(`Log entry written for index ${logEntry.index}: ${logEntry.status}`);
        } catch (error) {
            console.error(`Failed to write log entry: ${error.message}`);
        }
    }

    async processEntry(entry, index) {
        try {
            console.log(`Processing entry ${index}...`);

            // Find corresponding MongoDB entry
            const mongoEntry = await this.findMongoEntry(index);

            // Extract transaction hashes
            const inputTxHash = entry.txHash;
            const mongoTxHashes = mongoEntry.metadata?.specific?.tx_hashes || [];

            if (mongoTxHashes.length === 0) {
                throw new Error('No transaction hashes found in MongoDB entry');
            }

            // For now, use the first tx hash from MongoDB entry
            // You might want to modify this logic based on your requirements
            const mongoTxHash = mongoTxHashes[0];

            console.log(`Comparing blobs between ${inputTxHash} and ${mongoTxHash}`);

            // Get blob commitments for both transactions
            const [inputCommitments, mongoCommitments] = await Promise.all([
                this.getBlobCommitments(inputTxHash),
                this.getBlobCommitments(mongoTxHash)
            ]);

            // Compare commitments and create log entry
            const logEntry = this.compareBlobCommitments(
                inputCommitments,
                mongoCommitments,
                inputTxHash,
                mongoTxHash,
                index
            );

            // Write to log file
            await this.writeLogEntry(logEntry);

            return logEntry;

        } catch (error) {
            const errorLogEntry = {
                index,
                txHash1: entry.txHash,
                txHash2: 'N/A',
                timestamp: new Date().toISOString(),
                status: 'ERROR',
                message: error.message,
                blobCount1: 0,
                blobCount2: 0,
                commitmentsMatch: false
            };

            await this.writeLogEntry(errorLogEntry);
            console.error(`Error processing entry ${index}: ${error.message}`);
            return errorLogEntry;
        }
    }

    async run() {
        try {
            await this.initialize();

            // Initialize log file with configuration info
            const configInfo = this.maxIndexes ? ` (Processing first ${this.maxIndexes} entries)` : ' (Processing all entries)';
            const logHeader = `Blob Commitment Comparison Log - Started at ${new Date().toISOString()}${configInfo}\n${'='.repeat(80)}\n`;
            await fs.writeFile(CONFIG.logFilePath, logHeader);

            // Read input data
            const inputData = await this.readInputData();
            console.log(`Loaded ${inputData.length} entries from input file${this.maxIndexes ? ` (limited to ${this.maxIndexes})` : ''}`);

            // Process each entry
            const results = [];
            for (let i = 0; i < inputData.length; i++) {
                const result = await this.processEntry(inputData[i], i + 1);
                results.push(result);

                // Add a small delay to avoid overwhelming the RPC
                await new Promise(resolve => setTimeout(resolve, 100));
            }

            // Write summary
            const successCount = results.filter(r => r.status === 'SUCCESS' && r.commitmentsMatch).length;
            const errorCount = results.filter(r => r.status === 'ERROR').length;
            const mismatchCount = results.filter(r => r.status === 'SUCCESS' && !r.commitmentsMatch).length;

            const summary = `\n${'='.repeat(80)}\nSUMMARY:\nTotal entries processed: ${results.length}\nSuccessful matches: ${successCount}\nMismatches: ${mismatchCount}\nErrors: ${errorCount}\nCompleted at: ${new Date().toISOString()}\n`;

            await fs.appendFile(CONFIG.logFilePath, summary);
            console.log('Processing completed. Check the log file for detailed results.');
            console.log(`Summary: ${successCount} matches, ${mismatchCount} mismatches, ${errorCount} errors`);

        } catch (error) {
            console.error(`Script failed: ${error.message}`);
        } finally {
            if (this.mongoClient) {
                await this.mongoClient.close();
                console.log('MongoDB connection closed');
            }
        }
    }
}

// Parse command line arguments
function parseArgs() {
    const args = process.argv.slice(2);
    let maxIndexes = null;

    // Look for --count or -c argument
    const countIndex = args.findIndex(arg => arg === '--count' || arg === '-c');
    if (countIndex !== -1 && countIndex + 1 < args.length) {
        const countValue = parseInt(args[countIndex + 1]);
        if (isNaN(countValue) || countValue <= 0) {
            console.error('Error: Count must be a positive integer');
            process.exit(1);
        }
        maxIndexes = countValue;
    }

    // Look for --help or -h argument
    if (args.includes('--help') || args.includes('-h')) {
        console.log(`
Usage: node script.js [options]

Options:
  -c, --count <number>    Number of entries to process (default: all)
  -h, --help             Show this help message

Environment Variables Required:
  MONGO_URL              MongoDB connection URL
  DB_NAME                MongoDB database name
  RPC_URL                Ethereum RPC URL (Alchemy/Infura)
  INPUT_FILE_PATH        Path to the JSON input file

Examples:
  MONGO_URL="mongodb://localhost:27017" DB_NAME="mydb" RPC_URL="https://mainnet.infura.io/v3/KEY" INPUT_FILE_PATH="./data.json" node script.js
  MONGO_URL="mongodb://localhost:27017" DB_NAME="mydb" RPC_URL="https://mainnet.infura.io/v3/KEY" INPUT_FILE_PATH="./data.json" node script.js --count 10
        `);
        process.exit(0);
    }

    return { maxIndexes };
}

// Usage
async function main() {
    try {
        const { maxIndexes } = parseArgs();

        console.log('Starting Blob Commitment Matcher...');
        if (maxIndexes) {
            console.log(`Will process first ${maxIndexes} entries`);
        } else {
            console.log('Will process all entries');
        }

        const matcher = new BlobCommitmentMatcher(maxIndexes);
        await matcher.run();
    } catch (error) {
        console.error('Application error:', error.message);
        process.exit(1);
    }
}

// Run the script
if (require.main === module) {
    main().catch(console.error);
}

module.exports = BlobCommitmentMatcher;
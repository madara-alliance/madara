const fs = require('fs').promises;
const {MongoClient} = require('mongodb');
const {ethers} = require('ethers');

// Configuration
const CONFIG = {
    mongoUrl: '', // Update with your MongoDB URL
    dbName: '', // Update with your database name
    collectionName: 'jobs', // Update with your collection name
    ethereumRpcUrl: "", // Update with your Ethereum RPC URL (Alchemy/Infura)
    inputFilePath: '', // Path to your JSON input file
    logFilePath: './blob_comparison_log.txt'
};

class BlobCommitmentMatcher {
    constructor() {
        this.mongoClient = null;
        this.ethProvider = null;
        this.logEntries = [];
    }

    async initialize() {
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
            return JSON.parse(data);
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

            // Initialize log file
            const logHeader = `Blob Commitment Comparison Log - Started at ${new Date().toISOString()}\n${'='.repeat(80)}\n`;
            await fs.writeFile(CONFIG.logFilePath, logHeader);

            // Read input data
            const inputData = await this.readInputData();
            console.log(`Loaded ${inputData.length} entries from input file`);

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

            const summary = `\n${'='.repeat(80)}\nSUMMARY:\nTotal entries: ${results.length}\nSuccessful matches: ${successCount}\nMismatches: ${mismatchCount}\nErrors: ${errorCount}\nCompleted at: ${new Date().toISOString()}\n`;

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

// Usage
async function main() {
    const matcher = new BlobCommitmentMatcher();
    await matcher.run();
}

// Run the script
if (require.main === module) {
    main().catch(console.error);
}

module.exports = BlobCommitmentMatcher;

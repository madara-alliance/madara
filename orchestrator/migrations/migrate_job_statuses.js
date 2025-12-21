// MongoDB migration script for job status refactor
// Run with: mongosh mongodb://localhost:27017/orchestrator_db < migrate_job_statuses.js
// Or: mongosh --host <host> --port <port> --username <user> --password <pass> orchestrator_db < migrate_job_statuses.js

use orchestrator; // Change to your database name if different

print("Starting job status migration...");
print("Timestamp: " + new Date().toISOString());

// 1. PendingVerification → Processed
const pvResult = db.jobs.updateMany(
    { status: "PendingVerification" },
    { $set: { status: "Processed" } }
);
print("PendingVerification → Processed: " + pvResult.modifiedCount + " jobs updated");

// 2. PendingRetry → PendingRetryProcessing
const prResult = db.jobs.updateMany(
    { status: "PendingRetry" },
    { $set: { status: "PendingRetryProcessing" } }
);
print("PendingRetry → PendingRetryProcessing: " + prResult.modifiedCount + " jobs updated");

// 3. VerificationTimeout → PendingRetryVerification
const vtResult = db.jobs.updateMany(
    { status: "VerificationTimeout" },
    { $set: { status: "PendingRetryVerification" } }
);
print("VerificationTimeout → PendingRetryVerification: " + vtResult.modifiedCount + " jobs updated");

// 4. Failed → ProcessingFailed
const fResult = db.jobs.updateMany(
    { status: "Failed" },
    { $set: { status: "ProcessingFailed" } }
);
print("Failed → ProcessingFailed: " + fResult.modifiedCount + " jobs updated");

// Verify migration
print("\nVerifying migration - distinct statuses:");
const statuses = db.jobs.distinct("status");
printjson(statuses);

print("\nExpected statuses:");
print("  - Created");
print("  - LockedForProcessing");
print("  - Processed");
print("  - LockedForVerification");
print("  - Completed");
print("  - PendingRetryProcessing");
print("  - PendingRetryVerification");
print("  - ProcessingFailed");
print("  - VerificationFailed");

// Check for any remaining old statuses
print("\nChecking for old statuses...");
const oldStatuses = ["PendingVerification", "PendingRetry", "VerificationTimeout", "Failed"];
let hasOldStatuses = false;
for (const status of oldStatuses) {
    const count = db.jobs.countDocuments({ status: status });
    if (count > 0) {
        print("WARNING: " + count + " jobs still have old status: " + status);
        hasOldStatuses = true;
    }
}

if (!hasOldStatuses) {
    print("✓ No old statuses found");
}

print("\nMigration complete!");
print("Timestamp: " + new Date().toISOString());

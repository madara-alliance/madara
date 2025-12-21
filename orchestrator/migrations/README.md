# MongoDB Migration Scripts

This directory contains MongoDB migration scripts for the Orchestrator database.

## Job Status Migration

### Overview

The `migrate_job_statuses.js` script migrates job statuses from the old naming scheme to the new, more descriptive naming scheme implemented in the Phase 6 job state refactor.

### Status Changes

The migration updates the following statuses:

| Old Status | New Status |
|-----------|-----------|
| `PendingVerification` | `Processed` |
| `PendingRetry` | `PendingRetryProcessing` |
| `VerificationTimeout` | `PendingRetryVerification` |
| `Failed` | `ProcessingFailed` |

### Running the Migration

#### Option 1: Using mongosh with default local database

```bash
mongosh mongodb://localhost:27017/orchestrator_db < migrate_job_statuses.js
```

#### Option 2: Using mongosh with authentication

```bash
mongosh --host <host> --port <port> --username <user> --password <pass> orchestrator_db < migrate_job_statuses.js
```

#### Option 3: Using mongosh connection string

```bash
mongosh "mongodb://<user>:<password>@<host>:<port>/orchestrator_db?authSource=admin" < migrate_job_statuses.js
```

### Important Notes

1. **Database Name**: The script uses `orchestrator` as the database name by default. If your database has a different name, edit the first line of the script:
   ```javascript
   use your_database_name; // Change this line
   ```

2. **Idempotent**: The migration is idempotent and can be run multiple times safely. If a status has already been migrated, it will not be changed again.

3. **Verification**: The script includes verification steps that:
   - Shows all distinct statuses after migration
   - Lists expected statuses
   - Checks for any remaining old statuses
   - Provides warnings if old statuses are still present

4. **Backup Recommended**: While the migration is safe and idempotent, it's always recommended to backup your database before running any migration:
   ```bash
   mongodump --db orchestrator_db --out /path/to/backup
   ```

### Expected Output

A successful migration will show output similar to:

```
Starting job status migration...
Timestamp: 2025-12-21T10:30:00.000Z
PendingVerification → Processed: 15 jobs updated
PendingRetry → PendingRetryProcessing: 8 jobs updated
VerificationTimeout → PendingRetryVerification: 3 jobs updated
Failed → ProcessingFailed: 12 jobs updated

Verifying migration - distinct statuses:
[
  "Created",
  "LockedForProcessing",
  "Processed",
  "LockedForVerification",
  "Completed",
  "PendingRetryProcessing",
  "PendingRetryVerification",
  "ProcessingFailed",
  "VerificationFailed"
]

Expected statuses:
  - Created
  - LockedForProcessing
  - Processed
  - LockedForVerification
  - Completed
  - PendingRetryProcessing
  - PendingRetryVerification
  - ProcessingFailed
  - VerificationFailed

Checking for old statuses...
✓ No old statuses found

Migration complete!
Timestamp: 2025-12-21T10:30:01.234Z
```

### Troubleshooting

#### Connection Issues

If you encounter connection issues:
- Verify MongoDB is running
- Check the hostname, port, and credentials
- Ensure the database exists
- Check network/firewall settings

#### Permission Issues

If you get permission errors:
- Ensure your MongoDB user has read/write permissions on the database
- The user needs `update` permissions on the `jobs` collection

#### Old Statuses Still Present

If the verification reports old statuses still present after migration:
- Check if there are any application processes still running that might be creating jobs with old statuses
- Run the migration again (it's idempotent)
- If issues persist, check the MongoDB logs for errors

### Rollback

If you need to rollback the migration, you can reverse the changes:

```javascript
use orchestrator;

db.jobs.updateMany({ status: "Processed" }, { $set: { status: "PendingVerification" } });
db.jobs.updateMany({ status: "PendingRetryProcessing" }, { $set: { status: "PendingRetry" } });
db.jobs.updateMany({ status: "PendingRetryVerification" }, { $set: { status: "VerificationTimeout" } });
db.jobs.updateMany({ status: "ProcessingFailed" }, { $set: { status: "Failed" } });
```

**Warning**: Only perform a rollback if you're reverting to an older version of the orchestrator codebase that expects the old status names.

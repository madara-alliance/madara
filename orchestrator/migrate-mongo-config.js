// In this file you can configure migrate-mongo

const fs = require("node:fs");

// Mirror the Rust orchestrator's resolve_secret_from_file()
// (see crates/utils/src/env_utils.rs): if `${name}_FILE` is set, read the
// secret from that mounted file (K8s CSI Secrets Store Driver pattern). The
// file value takes precedence over the direct env var; an empty file is an
// error. Reading from a tmpfs file keeps the connection string out of
// /proc/<pid>/environ, crash dumps, and `kubectl describe pod`. Backward
// compatible: falls back to the plain env var when `${name}_FILE` is unset.
function resolveSecretFromFile(name) {
  const filePath = process.env[`${name}_FILE`];
  if (!filePath) return process.env[name];
  const value = fs.readFileSync(filePath, "utf8").trim();
  if (!value) {
    throw new Error(`Secret file '${filePath}' (set via ${name}_FILE) is empty`);
  }
  return value;
}

const config = {
  mongodb: {
    // TODO Change (or review) the url to your MongoDB:
    url:
      resolveSecretFromFile("MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL") ||
      "mongodb://localhost:27017",

    // TODO Change this to your database name:
    databaseName:
      process.env.MADARA_ORCHESTRATOR_DATABASE_NAME || "orchestrator",

    options: {
      // connectTimeoutMS: 3600000, // increase connection timeout to 1 hour
      // socketTimeoutMS: 3600000, // increase socket timeout to 1 hour
    },
  },

  // The migrations dir, can be an relative or absolute path. Only edit this when really necessary.
  migrationsDir: "migrations",

  // The mongodb collection where the applied changes are stored. Only edit this when really necessary.
  changelogCollectionName: "changelog",

  // The mongodb collection where the lock will be created.
  lockCollectionName: "changelog_lock",

  // The value in seconds for the TTL index that will be used for the lock.
  // Value of 0 will disable the feature.
  lockTtl: 60,

  // The file extension to create migrations and search for in migration dir
  migrationFileExtension: ".js",

  // Enable the algorithm to create a checksum of the file contents and use that in the comparison to determine
  // if the file should be run.  Requires that scripts are coded to be run multiple times.
  useFileHash: false,

  // Don't change this, unless you know what you're doing
  moduleSystem: "commonjs",
};

module.exports = config;

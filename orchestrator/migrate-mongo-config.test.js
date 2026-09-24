// Runnable check for the _FILE secret resolution in migrate-mongo-config.js.
// No test framework: uses Node's built-in runner + assert. Run with:
//   node --test
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const VAR = "MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL";

function loadConfig() {
  // Re-require fresh so env changes are picked up.
  delete require.cache[require.resolve("./migrate-mongo-config.js")];
  return require("./migrate-mongo-config.js");
}

function tmpFileWith(contents) {
  const p = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "mmc-")), "url");
  fs.writeFileSync(p, contents);
  return p;
}

test("_FILE variant takes precedence over the plain env var and is trimmed", () => {
  process.env[VAR] = "mongodb://env-url:27017";
  process.env[`${VAR}_FILE`] = tmpFileWith("  mongodb://file-url:27017\n");
  assert.strictEqual(loadConfig().mongodb.url, "mongodb://file-url:27017");
});

test("falls back to the plain env var when _FILE is unset", () => {
  delete process.env[`${VAR}_FILE`];
  process.env[VAR] = "mongodb://env-url:27017";
  assert.strictEqual(loadConfig().mongodb.url, "mongodb://env-url:27017");
});

test("an empty secret file is a hard error (never silently falls back)", () => {
  process.env[`${VAR}_FILE`] = tmpFileWith("   \n");
  assert.throws(() => loadConfig(), /is empty/);
});

# Bootstrap Snapshots

Bootstrap snapshots let operators start a Madara node from a pre-synced
database archive instead of syncing from genesis. A snapshot is a `.tar.gz`
archive whose root contains a Madara base path, including `db/` and
`.db-version`, plus a JSON manifest with the expected chain id, confirmed tip,
archive size, and SHA-256 checksum.

Use this for trusted operational bootstrap only. A snapshot imports database
state as-is, so only consume artifacts produced by an operator you trust.

## Snapshot Files

Madara writes and reads two files. Each file can be provided as a local
filesystem path, or imported over `http://` or `https://`.

| File | Description |
| ---- | ----------- |
| `<snapshot>.tar.gz` | Compressed archive of the Madara base path contents. |
| `<snapshot>.tar.gz.manifest.json` | Manifest used to verify the archive and validate the imported database tip. |

The manifest path is optional on both create and import. If omitted, Madara
appends `.manifest.json` to the archive path; for example, `mainnet.tar.gz`
defaults to `mainnet.tar.gz.manifest.json`.

## Create a Snapshot

Run the create command against an existing synced base path:

```bash
./target/production/madara \
  --full \
  --network mainnet \
  --base-path /var/lib/madara \
  --l1-endpoint "$ETHEREUM_API_URL" \
  --create-bootstrap-snapshot /srv/madara-snapshots/mainnet.tar.gz
```

Madara opens the database, reads the latest confirmed tip, creates a RocksDB
checkpoint, archives that checkpoint with `.db-version`, writes the manifest,
and exits before starting RPC, sync, or other node services.

Operational constraints:

- Pass the same chain selection flags used by the source node, such as
  `--network`, `--preset`, or `--chain-config-path`.
- The source database must end at a confirmed chain tip. Snapshot creation
  refuses preconfirmed tips.
- The archive and manifest output paths must not already exist.
- The archive and manifest output paths must be outside the source `--base-path`.
- This command creates a live-safe RocksDB checkpoint for its own open DB handle.
  It does not bypass RocksDB's process lock; stop any already-running Madara
  process that owns the same base path before running the create command.

To choose a non-default manifest path:

```bash
./target/production/madara \
  --full \
  --network mainnet \
  --base-path /var/lib/madara \
  --l1-endpoint "$ETHEREUM_API_URL" \
  --create-bootstrap-snapshot /srv/madara-snapshots/mainnet.tar.gz \
  --create-bootstrap-snapshot-manifest /srv/madara-snapshots/mainnet.manifest.json
```

## Publish a Snapshot

Publish the archive and manifest together. Keep their names stable enough that
new nodes can discover both files, and keep the block number visible in your
release metadata.

Recommended layout:

```text
snapshots/
  mainnet/
    latest.txt
    madara-mainnet-000123456.tar.gz
    madara-mainnet-000123456.tar.gz.manifest.json
```

`latest.txt` can contain the chosen archive filename. Madara does not read this
file directly; it is only an operator convention for deployment automation.

Before publishing, keep a local copy of the command output and inspect the
manifest:

```bash
jq . /srv/madara-snapshots/mainnet.tar.gz.manifest.json
```

## Import a Snapshot

Import a local snapshot before the target database opens:

```bash
./target/production/madara \
  --full \
  --network mainnet \
  --base-path /var/lib/madara \
  --l1-endpoint "$ETHEREUM_API_URL" \
  --bootstrap-snapshot /srv/madara-snapshots/mainnet.tar.gz
```

If the local manifest is not next to the archive at the default path, pass it
explicitly:

```bash
./target/production/madara \
  --full \
  --network mainnet \
  --base-path /var/lib/madara \
  --l1-endpoint "$ETHEREUM_API_URL" \
  --bootstrap-snapshot /srv/madara-snapshots/mainnet.tar.gz \
  --bootstrap-snapshot-manifest /srv/madara-snapshots/mainnet.manifest.json
```

To download and import a remote snapshot, use `--bootstrap-snapshot-url`:

```bash
./target/production/madara \
  --full \
  --network mainnet \
  --base-path /var/lib/madara \
  --l1-endpoint "$ETHEREUM_API_URL" \
  --bootstrap-snapshot-url https://snapshots.example/madara/mainnet.tar.gz
```

If `--bootstrap-snapshot-manifest-url` is omitted, Madara appends
`.manifest.json` to the archive URL path. The example above defaults to:

```text
https://snapshots.example/madara/mainnet.tar.gz.manifest.json
```

Pass the manifest URL explicitly when your storage layer uses signed URLs,
redirects, or a different object name:

```bash
./target/production/madara \
  --full \
  --network mainnet \
  --base-path /var/lib/madara \
  --l1-endpoint "$ETHEREUM_API_URL" \
  --bootstrap-snapshot-url https://snapshots.example/madara/mainnet.tar.gz?archive-token=... \
  --bootstrap-snapshot-manifest-url https://snapshots.example/madara/mainnet.manifest.json?manifest-token=...
```

Import behavior:

- The target `--base-path` must be missing or empty.
- For URL imports, Madara downloads the manifest and archive into a temporary
  directory first.
- Madara verifies the manifest format version, configured chain id, archive
  size, and archive SHA-256 before extracting.
- Madara extracts into a staging directory and atomically moves it into place.
- Archive entries must be regular files or directories; links and path traversal
  entries are rejected.
- After opening the imported database, Madara validates the confirmed chain tip,
  block hash, and state root recorded in the manifest.

After validation, the node continues normal startup and syncs forward from the
snapshot tip.

## Environment Variables

The snapshot flags also have environment variable forms:

| CLI flag | Environment variable |
| -------- | -------------------- |
| `--bootstrap-snapshot` | `MADARA_BOOTSTRAP_SNAPSHOT` |
| `--bootstrap-snapshot-url` | `MADARA_BOOTSTRAP_SNAPSHOT_URL` |
| `--bootstrap-snapshot-manifest` | `MADARA_BOOTSTRAP_SNAPSHOT_MANIFEST` |
| `--bootstrap-snapshot-manifest-url` | `MADARA_BOOTSTRAP_SNAPSHOT_MANIFEST_URL` |
| `--create-bootstrap-snapshot` | `MADARA_CREATE_BOOTSTRAP_SNAPSHOT` |
| `--create-bootstrap-snapshot-manifest` | `MADARA_CREATE_BOOTSTRAP_SNAPSHOT_MANIFEST` |

## Troubleshooting

`base path is not empty`

The target already contains files. Move it aside or choose a fresh `--base-path`.
Madara refuses to overwrite an existing database during import.

`checksum mismatch`

The archive does not match the manifest. Re-copy both files from the publisher
and verify that automation is not mixing archive and manifest versions.

`snapshot is for chain id ...`

The manifest chain id does not match the node configuration. Check `--network`,
`--preset`, `--chain-config-path`, and any chain config overrides.

`must end at confirmed block`

Snapshot creation found a preconfirmed tip, or import validation opened an
archive whose stored tip is preconfirmed. Let the source node seal a confirmed
block, or clear/avoid preconfirmed state before creating the snapshot.

`output must be outside source base path`

Write snapshots to a separate directory such as `/srv/madara-snapshots`, not
under `/var/lib/madara`.

## Current Limitations

- Snapshot trust is external to Madara. Madara verifies integrity against the
  manifest, but it does not prove that the manifest itself is honest.
- Snapshot compatibility follows the database format supported by the running
  binary. Normal database migrations still run on open when needed.

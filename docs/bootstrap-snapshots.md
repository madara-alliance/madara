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

The repository includes a publisher script that creates this layout and
verifies the archive against the manifest before moving the files into place:

```bash
scripts/bootstrap-snapshot-publish.sh \
  --madara-bin ./target/release/madara \
  --network mainnet \
  --base-path /var/lib/madara \
  --output-dir /srv/madara-snapshots
```

The script writes `latest.txt` only after the canonical archive and manifest
are present. It does not sync the source node or stop an already-running
service. Run it against a stopped source node or a dedicated standby node whose
database is already synced.

For GitHub Actions operations, use
`.github/workflows/publish-bootstrap-snapshot.yml`. The workflow builds the
Madara binary, runs the publisher script on a self-hosted runner, and either
leaves the generated files on the runner or publishes them to S3. GitHub
artifact upload is available as an explicit opt-in for small test snapshots,
but should not be used for production mainnet archives.

The workflow intentionally requires a self-hosted runner because production
snapshots need a persistent synced database. For an AWS-hosted database, run the
GitHub runner on EC2 in the same AWS environment and expose the Madara base path
as a local mount, for example an attached EBS volume, a cloned EBS snapshot
volume, or another filesystem path that contains the synced `db/` and
`.db-version`. Ephemeral GitHub runners should not download or rebuild a
current mainnet database from scratch.

The snapshot source must not be locked by a running Madara process. In AWS, the
usual safe patterns are:

- Run the publisher on a dedicated standby node whose Madara process is stopped
  during snapshot creation.
- Create and attach an EBS clone of the synced volume to the snapshot runner,
  then create the archive from the clone.
- Schedule a maintenance window that stops Madara, runs the publisher, and
  starts Madara again.

Manual dry run:

```bash
gh workflow run publish-bootstrap-snapshot.yml \
  -f network=mainnet \
  -f base_path=/var/lib/madara \
  -f output_dir=/srv/madara-snapshots \
  -f runner_label=madara-snapshot \
  -f publish=false
```

S3 publish run:

```bash
gh workflow run publish-bootstrap-snapshot.yml \
  -f network=mainnet \
  -f base_path=/var/lib/madara \
  -f output_dir=/srv/madara-snapshots \
  -f runner_label=madara-snapshot \
  -f publish=true \
  -f s3_prefix=snapshots
```

Recommended repository settings for scheduled publishing:

| Name | Type | Description |
| ---- | ---- | ----------- |
| `MADARA_BOOTSTRAP_SNAPSHOT_SCHEDULED` | variable | Set to `true` to enable the workflow schedule. |
| `MADARA_BOOTSTRAP_SNAPSHOT_RUNNER_LABEL` | variable | AWS self-hosted runner label. Defaults to `madara-snapshot`. |
| `MADARA_BOOTSTRAP_SNAPSHOT_NETWORK` | variable | Network for scheduled runs. Defaults to `mainnet`. |
| `MADARA_BOOTSTRAP_SNAPSHOT_BASE_PATH` | variable | Synced source base path on the runner. Defaults to `/var/lib/madara`. |
| `MADARA_BOOTSTRAP_SNAPSHOT_OUTPUT_DIR` | variable | Local output directory on the runner. Defaults to `/srv/madara-snapshots`. |
| `MADARA_BOOTSTRAP_SNAPSHOT_PUBLISH` | variable | Set to `true` for scheduled S3 publishing. |
| `MADARA_BOOTSTRAP_SNAPSHOT_BUCKET` | variable | Destination S3 bucket for published snapshots. |
| `MADARA_BOOTSTRAP_SNAPSHOT_PREFIX` | variable | Destination object prefix. Defaults to `snapshots`. |
| `MADARA_BOOTSTRAP_SNAPSHOT_PUBLIC_BASE_URL` | variable | Optional public URL root used in workflow summaries. |
| `AWS_REGION` | variable or secret | AWS region used by the OIDC publishing role. |
| `AWS_BOOTSTRAP_SNAPSHOT_ROLE_ARN` | secret | OIDC role assumed by the workflow for S3 writes. |

The AWS role assumed by the workflow needs write access to
`s3://$MADARA_BOOTSTRAP_SNAPSHOT_BUCKET/$MADARA_BOOTSTRAP_SNAPSHOT_PREFIX/<network>/`
for the archive, manifest, and `latest.txt`. If the bucket uses KMS encryption,
the role also needs the matching KMS permissions.

Before publishing manually, keep a local copy of the command output and inspect
the manifest:

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

# October 2026 emSign trust remediation

This change uses published upstream TLS fixes. There are no local copies or
custom patches of rustls-webpki or webpki-roots. The existing Cairo, RocksDB,
JSON-RPC and OmniQueue forks predate this remediation and retain their application
behavior. Rebuild and validate each deployed revision before promoting an image.

## Dependency changes

| Dependency | Change | Reason |
| --- | --- | --- |
| rustls-webpki | 0.103.16 | Upstream compromised-key distrust, including explicitly trusted intermediates |
| webpki-roots / webpki-root-certs | 1.0.10 | Published root-store removal; webpki-roots 0.26.11 delegates to 1.0.10 |
| aws-lc-rs / aws-lc-sys | 1.18.2 / 0.46.0 | Required by the fixed verifier |
| mongodb | 2.8.2 → 3.5.2 | Move the orchestrator onto the rustls 0.23 verifier already used by Madara's external DB |
| testcontainers | 0.18.0 → 0.21.1 | Move test Docker connections to Bollard 0.17 / rustls 0.23 |
| omniqueue | SQS feature only | Remove unused GCP, Redis and RabbitMQ clients, including the old gRPC TLS stack |
| jsonrpsee clients | Disable unused TLS features | Production use is hardcoded local HTTP; WS clients are local tests only |

The MongoDB driver upgrade uses its action-builder API. Filters, update documents,
sorts, limits, indexes and BSON 2 serialization remain the same; chrono and UUID
features move from the driver manifest to the BSON dependency. MongoDB integration
and locking tests are promotion gates, because this is a major driver upgrade.

The JSON-RPC server and its existing Starknet subscription-message fork stay at
the current revision. Direct HTTP/WS client dependencies disable default TLS
features. HTTPS/WSS URLs are rejected, never downgraded. External HTTPS clients
continue to use TLS. Do not enable these old client TLS features for a new remote
endpoint; upgrade that client first.

## AWS compatibility dependency: explicit limitation

The published AWS SDK still includes an optional Hyper 0.14 / rustls 0.21 client,
which brings **unpatched rustls-webpki 0.101.7** into Cargo.lock. This change does
not claim that this crate is fixed or suppress its advisory. A simple upgrade to
the latest SDK does not remove that compatibility dependency.

The orchestrator explicitly selects AWS behavior version 2025-08-07. With
`default-https-client`, that selects Hyper 1 / rustls 0.23 and the patched
rustls-webpki 0.103.16. OmniQueue's existing `load_from_env()` selects the latest
behavior and therefore the same modern connector. The regression suite checks
both selections with the legacy feature also enabled, and checks exactly the
application's locked AWS transport versions. `check-lock.py` rejects any non-AWS
consumer of the old verifier or a reintroduced 0.102 verifier/0.25 root store.

This is a reviewed runtime mitigation with a remaining compatibility dependency,
not an advisory-free dependency graph. Do not choose an older AWS behavior version
or instantiate `hyper_014` clients. Removing the compatibility dependency completely
requires parent libraries to stop enabling it; it does not justify another local
TLS fork.

## Image and tool changes

- Sanitize Debian builder and runtime CA stores before tool downloads. This also
  removes AAA Certificate Services and can affect legitimate chains; validate
  required endpoints before promotion.
- Use Python 3.11, pip 26.2.2 and certifi 2026.10.19. Bootstrap pinned, hash-checked
  wheels with the sanitized system store and install them offline. Replace the
  sequencer requirements file's older certifi pin before resolving with the fixed
  constraints, then check both Python and pip CA bundles.
- Install Node 22.23.4 from the official x64 archive with its pinned SHA256.
- Keep locked Cargo builds and fresh manual Docker builds, preserving the existing
  application build profiles.

## Validation

```bash
python3.11 security/check-lock.py
cargo test --locked --manifest-path security/tls-regression/Cargo.toml
cargo metadata --locked --no-deps --format-version 1 >/dev/null
cargo fmt --all -- --check
cargo check --locked
```

The isolated suite tests the published verifier against 12 public incident chains,
the same chains with compromised intermediates explicitly trusted, an unrelated
private CA, and the bundled root stores. A fixed verification date (2026-10-20)
and explicit distrust-error assertions prevent expiry from causing false passes.
It also checks AWS modern-client selection and loopback clients' rejection of TLS
URLs. Fixtures are public certificates captured on 2026-10-21, not library source.

These checks run on PRs and before manual image publication. Full application and
integration tests, required endpoint connectivity, and runtime image checks remain
promotion gates. No test here establishes that a host, cluster or third-party image
is fully remediated.

## Build a candidate without deploying

Use the exact reviewed patch branch. Set only the component you need to true:

```bash
gh workflow run manual-build-docker-images.yml --repo madara-alliance/madara \
  --ref REVIEWED_PATCH_BRANCH \
  -f build-madara=true -f build-orchestrator=false \
  -f build-bootstrapper=false -f build-bootstrapper-v2=false
```

For orchestrator, swap the first two booleans. A non-main branch publishes
`ghcr.io/madara-alliance/{madara,orchestrator}:manual-<commit-sha7>` and does not move
`manual-latest`. Resolve and record the registry digest after a successful build;
deploy using `@sha256:...`, never overwrite an existing deployed tag. The manual
workflow does not deploy. Do not run the orchestrator entrypoint with production
credentials as a smoke test: it can execute database migrations.

## Sources

- [Mozilla incident](https://bugzilla.mozilla.org/show_bug.cgi?id=2083449)
- [Public certificate fixtures](https://emudhra.we-have-a-problem.com/)
- [Upstream verifier fix](https://github.com/rustls/webpki/pull/546)
- [RustSec verifier advisory](https://rustsec.org/advisories/RUSTSEC-2026-0382.html)
- [RustSec roots advisory](https://rustsec.org/advisories/RUSTSEC-2026-0381.html)
- [RustSec root-certs advisory](https://rustsec.org/advisories/RUSTSEC-2026-0383.html)
- [Node 22.23.4](https://github.com/nodejs/node/releases/tag/v22.23.4)
- [Python incident guidance](https://discuss.python.org/t/information-for-python-users-regarding-emudhra-emsign-ca-certificate-security-incident/109576)

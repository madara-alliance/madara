# October 2026 emSign trust remediation

This patch addresses CVE-2026-111943 in the Madara and orchestrator build/runtime
paths. It is not a claim that a cluster, host, or third-party image is safe.
Rebuild and validate each deployed application revision before rolling it out.

## What changes

- Upgrade rustls-webpki 0.103 to 0.103.16 and the 1.x bundled root packages to
  1.0.10. The verifier upgrade also requires aws-lc-rs 1.18 / aws-lc-sys 0.46.
- Backport the upstream eight-SPKI distrust list to rustls-webpki 0.101.7 and
  0.102.8. These are live dependencies of older HTTP, MongoDB and gRPC clients;
  changing only 0.103 is insufficient. Check every candidate certificate and
  trust anchor during path construction, preserving alternative valid paths.
  Return the existing `CertRevoked` error to retain the legacy public API.
- Remove the emSign ECC G3 root from the independent webpki-roots 0.25.4 store.
  The 0.26.11 wrapper resolves the patched 1.0.10 parent.
- Disable four incident-related roots in Debian build/runtime CA stores before
  downloading tools. This includes AAA Certificate Services and can break
  legitimate services still dependent on that legacy root. Test required
  upstream endpoints before promotion; do not bypass certificate validation.
- Build with Python 3.11, pip 26.2.2 and certifi 2026.10.19. Check both certifi
  bundles after Cairo dependency installation. Bootstrap the patched pip/certifi wheels with the sanitized system CA file and
  pinned SHA256 hashes, then install them offline. This works with both old pip
  and versions that removed the legacy-certs option.
- Install Node 22.23.4 from the official x64 archive, checking its pinned SHA256
  against the published release checksum. Preserve existing migration behavior.
- Use locked Cargo dependencies and fresh manual Docker builds. Copy the vendored
  sources into dependency build stages: non-workspace patch crates are omitted
  from cargo-chef recipes. Preserve existing compilation caching and build profiles.

## Review the vendored backport

`vendor/provenance.json` records the original crates.io archive checksums.
`vendor/backport.patch` is the small text diff from those exact published sources.
The only added verifier data are the eight `src/data/distrusted/*.spki.der` files
copied byte-for-byte from rustls-webpki 0.103.16. Licenses and build-required library sources are preserved. Upstream test fixtures,
CI files and benchmarks are omitted to keep this import reviewable; manifest test
targets are disabled. The independent incident regression suite remains enabled. A scoped lifetime-syntax lint allowance is necessary because
Cargo does not cap lints for local path dependencies on Rust 1.89.

The older packages deliberately retain their API-compatible version numbers.
A scanner may still report their upstream versions: review the backport and the
test evidence, rather than suppressing the advisory for unpatched registry copies.
Remove these backports when all parent clients can use a supported verifier.

## Validation

```bash
python3.11 security/check-lock.py
cargo test --locked --manifest-path security/tls-regression/Cargo.toml
cargo metadata --locked --no-deps --format-version 1 >/dev/null
cargo fmt --all -- --check
```

The isolated test crate avoids Cairo/LLVM and tests all three verifier versions:
12 public chains, the same 12 with their compromised intermediate directly trusted,
an unrelated generated private-CA chain, and all bundled root stores. The fixed
verification date is 2026-10-20; tests require explicit distrust errors, so expiry
or network outages cannot produce a false success. Fixtures contain only public
certificates, downloaded from the incident diagnostic service on 2026-10-21.

The PR workflow runs these checks on any base branch, including revision-specific
backports. The manual image workflow repeats the regression before publication.
Full application compilation, required endpoint connectivity, migration compatibility,
and runtime image checks are additional promotion gates, not implied by this suite.

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
- [Upstream verifier backport](https://github.com/rustls/webpki/pull/546)
- [RustSec verifier advisory](https://rustsec.org/advisories/RUSTSEC-2026-0382.html)
- [RustSec roots advisory](https://rustsec.org/advisories/RUSTSEC-2026-0381.html)
- [RustSec root-certs advisory](https://rustsec.org/advisories/RUSTSEC-2026-0383.html)
- [Node 22.23.4](https://github.com/nodejs/node/releases/tag/v22.23.4)
- [Python incident guidance](https://discuss.python.org/t/information-for-python-users-regarding-emudhra-emsign-ca-certificate-security-incident/109576)

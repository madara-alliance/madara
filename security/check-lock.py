#!/usr/bin/env python3
"""Check upstream TLS versions and constrain the remaining AWS compatibility stack."""
from collections import defaultdict
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parent.parent
lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
packages = {(p["name"], p["version"]): p for p in lock["package"]}
by_name = defaultdict(list)
for key in packages:
    by_name[key[0]].append(key)

allowed = {
    "rustls-webpki": {"0.101.7", "0.103.16"},
    "webpki-roots": {"0.26.11", "1.0.10"},
    "webpki-root-certs": {"1.0.10"},
}
for (name, version), package in packages.items():
    if name in allowed:
        if version not in allowed[name]:
            raise SystemExit(f"Unreviewed TLS package: {name} {version}")
        if package.get("source") != "registry+https://github.com/rust-lang/crates.io-index" or not package.get("checksum"):
            raise SystemExit(f"TLS packages must be unmodified crates.io releases: {name} {version}")
if (ROOT / "security/vendor").exists():
    raise SystemExit("Local TLS backports must not be reintroduced")

# AWS still publishes an optional Hyper 0.14 / rustls 0.21 compatibility client.
# It is not fixed. Reject any other consumer, rather than globally suppressing it.
parents = defaultdict(set)
for key, package in packages.items():
    for dep in package.get("dependencies", []):
        parts = dep.split()
        candidates = by_name[parts[0]]
        if len(parts) > 1 and not parts[1].startswith("("):
            candidates = [k for k in candidates if k[1] == parts[1]]
        if len(candidates) != 1:
            raise SystemExit(f"Ambiguous lockfile dependency: {key}: {dep}")
        parents[candidates[0]].add(key)
legacy = {("rustls-webpki", "0.101.7"), ("rustls", "0.21.12"),
          ("tokio-rustls", "0.24.1"), ("hyper-rustls", "0.24.2")}
aws_boundary = ("aws-smithy-http-client", "1.1.5")
for key in legacy:
    if key in packages and not parents[key] <= legacy | {aws_boundary}:
        raise SystemExit(f"Legacy TLS has a non-AWS consumer: {key}: {parents[key]}")

# The isolated regression must exercise exactly the application's AWS transport
# releases. A future upgrade requires reviewing its default TLS selection again.
regression = tomllib.loads((ROOT / "security/tls-regression/Cargo.lock").read_text())
regression_packages = {(p["name"], p["version"]) for p in regression["package"]}
for key in [("aws-smithy-runtime", "1.9.5"), aws_boundary,
            ("rustls", "0.23.35"), ("rustls-webpki", "0.103.16")]:
    if by_name[key[0]] != [key] and key[0] not in {"rustls", "rustls-webpki"}:
        raise SystemExit(f"Review AWS default-client selection for {key[0]}")
    if key not in packages or key not in regression_packages:
        raise SystemExit(f"Application/regression transport versions differ: {key}")

workspace = manifest["workspace"]["dependencies"]
if set(workspace["jsonrpsee"].get("features", [])) & {"client", "full", "http-client", "ws-client"}:
    raise SystemExit("Use the TLS-free loopback clients instead of jsonrpsee's broad client feature")
for name in ["jsonrpsee-http-client", "jsonrpsee-ws-client"]:
    dep = workspace[name]
    if dep.get("default-features", True) or dep.get("features"):
        raise SystemExit(f"Loopback client must not enable the old TLS backend: {name}")
queue = workspace["omniqueue"]
if queue.get("default-features", True) or queue.get("features") != ["sqs"]:
    raise SystemExit("Only the used OmniQueue SQS backend is reviewed")

print("Published TLS fixes verified; legacy WebPKI is confined to AWS compatibility dependencies.")
print("Run the client-selection regression; this lockfile check alone is not a safety attestation.")

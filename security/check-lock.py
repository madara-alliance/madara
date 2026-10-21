#!/usr/bin/env python3
"""Fail closed if a workspace TLS dependency escapes the reviewed versions."""
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parent.parent
lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
allowed = {
    "rustls-webpki": {"0.101.7", "0.102.8", "0.103.16"},
    "webpki-roots": {"0.25.4", "0.26.11", "1.0.10"},
    "webpki-root-certs": {"1.0.10"},
}
legacy = {("rustls-webpki", "0.101.7"), ("rustls-webpki", "0.102.8"), ("webpki-roots", "0.25.4")}
patches = manifest["patch"]["crates-io"].values()
for package in lock["package"]:
    name, version = package["name"], package["version"]
    if name not in allowed:
        continue
    if version not in allowed[name]:
        raise SystemExit(f"Unreviewed TLS package: {name} {version}")
    if (name, version) in legacy:
        path = f"security/vendor/{name}-{version}"
        if "source" in package or not any(p.get("package") == name and p.get("path") == path for p in patches):
            raise SystemExit(f"Legacy TLS package must use the reviewed local backport: {name} {version}")
print("Workspace TLS lockfile and legacy backport paths are valid")

#!/usr/bin/env python3
"""Remove incident-related roots from a Debian image, or inspect a PEM bundle.

Only --debian modifies files. Run it while building an image, before downloading
build tools. Does not disable TLS verification or remove unrelated/private CAs.
"""
import argparse
import hashlib
from pathlib import Path
import re
import ssl
import subprocess

BLOCKED = {
    "86a1ecba089c4a8d3bbe2734c612ba341d813e043cf9e8a862cd5c57a36bbe6b",
    "d7a7a0fb5d7e2731d771e9484ebcdef71d5f0c3e0a2948782bc83ee0ea699ef4",
    "7dd78d5f4f13459a83dff9abbba62edbaf6f2d102bf257fd712f4d9f2746ed8d",
    "f2edaaff3d9b701fb4d14c9f285f8c0bad90fc3bcc1fa07d47f4718edaba34a5",
}


def fingerprints(path):
    pem = path.read_text()
    certs = re.findall(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----", pem, re.S)
    if not certs:
        raise ValueError(f"No certificates in {path}")
    return {hashlib.sha256(ssl.PEM_cert_to_DER_cert(c)).hexdigest() for c in certs}


def check(path):
    found = fingerprints(path) & BLOCKED
    if found:
        raise SystemExit(f"Distrusted roots remain in {path}: {sorted(found)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--debian", action="store_true")
    mode.add_argument("--check-bundle", type=Path)
    args = parser.parse_args()
    if args.check_bundle:
        check(args.check_bundle)
        return
    config = Path("/etc/ca-certificates.conf")
    lines = []
    for line in config.read_text().splitlines():
        if line and not line.startswith(("#", "!")):
            path = Path("/usr/share/ca-certificates") / line
            if fingerprints(path) & BLOCKED:
                line = "!" + line
        lines.append(line)
    config.write_text("\n".join(lines) + "\n")
    subprocess.run(["update-ca-certificates", "--fresh"], check=True)
    check(Path("/etc/ssl/certs/ca-certificates.crt"))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Bootstrap patched pip/certifi without relying on the existing pip trust store."""
import hashlib
from pathlib import Path
import ssl
import subprocess
import sys
import tempfile
import urllib.request

# Official PyPI wheels; hashes are pinned independently of the download response.
WHEELS = [
    (
        "https://files.pythonhosted.org/packages/3f/06/3e4420302363fbb71be3c7f35ce042f54bfca330e3e66b3a2e3c4cca579f/pip-26.2.2-py3-none-any.whl",
        "fd7f4dfa8190e014013b9f08f3e973f4e4c26ffd179a204895533a7254813a70",
    ),
    (
        "https://files.pythonhosted.org/packages/c4/51/29046e48f693c54e5b7412216d447a1d8362c9653148d98c02cdf65fa733/certifi-2026.10.19-py3-none-any.whl",
        "42d35340cc5b9ba78e818ddbff0c5b1c2b779de448a340fd1b989f5fa8670e1b",
    ),
]


def main():
    bundle = "/etc/ssl/certs/ca-certificates.crt"
    subprocess.run(
        [sys.executable, str(Path(__file__).with_name("distrust-emsign.py")), "--check-bundle", bundle],
        check=True,
    )
    # An explicit cafile uses only the sanitized OS roots, not pip's old bundle.
    context = ssl.create_default_context(cafile=bundle)
    with tempfile.TemporaryDirectory() as tmp:
        paths = []
        for url, expected_hash in WHEELS:
            with urllib.request.urlopen(url, context=context, timeout=30) as response:
                data = response.read()
            if hashlib.sha256(data).hexdigest() != expected_hash:
                raise SystemExit("Python bootstrap wheel checksum mismatch")
            path = Path(tmp) / url.rsplit("/", 1)[1]
            path.write_bytes(data)
            paths.append(str(path))
        subprocess.run(
            [sys.executable, "-m", "pip", "--isolated", "--disable-pip-version-check",
             "install", "--no-index", "--no-deps", "--upgrade", *paths],
            check=True,
        )


if __name__ == "__main__":
    main()

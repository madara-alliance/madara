#!/bin/sh
# Fetch the published Madara contract artifacts from ghcr.io WITHOUT a Docker daemon.
#
# The artifacts image (ghcr.io/madara-alliance/artifacts) is built FROM scratch and contains a
# single file, /artifacts.tar.gz, which itself unpacks `build-artifacts/...` relative to the
# repository root. This script downloads the image layer over plain HTTPS using the public GHCR
# registry API (anonymous pull token), then extracts the artifacts in place.
#
# Usage: ./scripts/fetch-artifacts.sh [version]
#   version defaults to `current_version` in .artifact-versions.yml
#
# Requirements: curl, tar, gzip (POSIX sh; no docker, jq, crane, or oras needed).
set -eu

REGISTRY="ghcr.io"
REPOSITORY="madara-alliance/artifacts"

# Resolve the repository root from this script's location.
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  VERSION=$(sed -n 's/^current_version:[[:space:]]*//p' "$ROOT/.artifact-versions.yml" | head -n 1)
fi
if [ -z "$VERSION" ]; then
  echo "error: could not determine artifact version from $ROOT/.artifact-versions.yml" >&2
  exit 1
fi

IMAGE="$REGISTRY/$REPOSITORY:$VERSION"
echo "Fetching artifacts from $IMAGE (no Docker daemon required)..."

TMPDIR_FETCH=$(mktemp -d)
trap 'rm -rf "$TMPDIR_FETCH"' EXIT INT TERM

# 1. Anonymous pull token for the public image.
TOKEN=$(curl -fsSL "https://$REGISTRY/token?scope=repository:$REPOSITORY:pull" |
  sed -n 's/.*"token"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$TOKEN" ]; then
  echo "error: failed to obtain a pull token from https://$REGISTRY/token" >&2
  exit 1
fi

ACCEPT="application/vnd.oci.image.manifest.v1+json"
ACCEPT="$ACCEPT,application/vnd.docker.distribution.manifest.v2+json"
ACCEPT="$ACCEPT,application/vnd.oci.image.index.v1+json"
ACCEPT="$ACCEPT,application/vnd.docker.distribution.manifest.list.v2+json"

fetch_manifest() {
  curl -fsSL -H "Authorization: Bearer $TOKEN" -H "Accept: $ACCEPT" \
    "https://$REGISTRY/v2/$REPOSITORY/manifests/$1"
}

# 2. Image manifest. If the tag points at a multi-arch index, follow the first entry (the
#    artifacts image is platform-independent data).
MANIFEST=$(fetch_manifest "$VERSION")
case "$MANIFEST" in
*image.index* | *manifest.list*)
  CHILD_DIGEST=$(printf '%s' "$MANIFEST" | grep -o 'sha256:[a-f0-9]\{64\}' | head -n 1)
  MANIFEST=$(fetch_manifest "$CHILD_DIGEST")
  ;;
esac

# 3. Layer digests: everything after the "layers" key (the config digest comes before it).
LAYER_DIGESTS=$(printf '%s' "$MANIFEST" | tr -d ' \t\n' | sed 's/.*"layers"://' |
  grep -o 'sha256:[a-f0-9]\{64\}')
if [ -z "$LAYER_DIGESTS" ]; then
  echo "error: could not find any layers in the manifest for $IMAGE" >&2
  exit 1
fi

# 4. Download and unpack each layer (gzipped tar). The layer filesystem contains artifacts.tar.gz.
for DIGEST in $LAYER_DIGESTS; do
  echo "Downloading layer $DIGEST..."
  curl -fsSL -H "Authorization: Bearer $TOKEN" \
    "https://$REGISTRY/v2/$REPOSITORY/blobs/$DIGEST" -o "$TMPDIR_FETCH/layer.tar.gz"
  tar -xzf "$TMPDIR_FETCH/layer.tar.gz" -C "$TMPDIR_FETCH"
  rm -f "$TMPDIR_FETCH/layer.tar.gz"
done

if [ ! -f "$TMPDIR_FETCH/artifacts.tar.gz" ]; then
  echo "error: artifacts.tar.gz not found inside the image layers of $IMAGE" >&2
  exit 1
fi

# 5. Extract build-artifacts/... into the repository root.
tar -xzf "$TMPDIR_FETCH/artifacts.tar.gz" -C "$ROOT"

echo "Done. Artifacts extracted under $ROOT/build-artifacts/"

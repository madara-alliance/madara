#!/bin/bash
# Create and upload base DB fixture for migration testing
#
# Usage:
#   ./scripts/create-base-db.sh [VERSION] [BLOCKS]
#
# Examples:
#   ./scripts/create-base-db.sh 8 50      # Create v8 DB with 50 blocks
#   ./scripts/create-base-db.sh 9 100     # Create v9 DB with 100 blocks
#
# Prerequisites:
#   - Docker installed and authenticated to ghcr.io
#   - Rust toolchain installed
#
# To authenticate with ghcr.io:
#   echo $GITHUB_TOKEN | docker login ghcr.io -u YOUR_GITHUB_USERNAME --password-stdin
#
# Environment variables:
#   GITHUB_TOKEN - Required for docker login and making package public
#                  (needs 'write:packages' and 'read:org' scopes)
#
# The script will:
#   1. Build madara
#   2. Sync specified blocks from Sepolia
#   3. Package the DB as a Docker image
#   4. Push to ghcr.io/madara-alliance/db-fixture:v{VERSION}

set -e

VERSION="${1:-8}"
BLOCKS="${2:-50}"
DB_PATH="/tmp/madara-base-db-v${VERSION}"
IMAGE="ghcr.io/madara-alliance/db-fixture:v${VERSION}"

echo "============================================"
echo "  Create Base DB Fixture"
echo "============================================"
echo "  Version: ${VERSION}"
echo "  Blocks:  ${BLOCKS}"
echo "  Image:   ${IMAGE}"
echo "============================================"
echo ""

# Check prerequisites
if ! command -v docker &> /dev/null; then
    echo "❌ Docker not found. Install it first."
    exit 1
fi

# Check ghcr.io authentication
# if ! docker login ghcr.io --get-login &> /dev/null; then
#     echo "❌ Not authenticated to ghcr.io"
#     echo ""
#     echo "To authenticate, run:"
#     echo "  echo \$GITHUB_TOKEN | docker login ghcr.io -u YOUR_GITHUB_USERNAME --password-stdin"
#     echo ""
#     echo "Your GITHUB_TOKEN needs 'write:packages' permission."
#     exit 1
# fi
# echo "✅ Authenticated to ghcr.io"

# Clean up any existing DB
rm -rf "${DB_PATH}"
mkdir -p "${DB_PATH}"

# Build madara
echo "🔨 Building madara..."
cd "$(dirname "$0")/../madara"
cargo build -p madara

# Sync blocks
echo "🔄 Syncing ${BLOCKS} blocks from Sepolia..."
MADARA_BIN="${CARGO_TARGET_DIR:-./target}/debug/madara"
timeout 60 "${MADARA_BIN}" \
    --name base-db-creator \
    --base-path "${DB_PATH}" \
    --network mainnet \
    --full \
    --no-l1-sync \
    --sync-stop-at "${BLOCKS}" 2>&1 || true

# Verify DB was created
if [ ! -f "${DB_PATH}/.db-version" ]; then
    echo "❌ Failed to create DB"
    exit 1
fi

DB_VERSION=$(cat "${DB_PATH}/.db-version")
echo "✅ DB created with version: ${DB_VERSION}"

# Package as tarball
echo "📦 Packaging..."
TARBALL="/tmp/db-fixtures-v${VERSION}.tar.gz"
tar -czf "${TARBALL}" -C "${DB_PATH}" .
ls -lh "${TARBALL}"

# Create minimal Dockerfile
DOCKER_DIR="/tmp/db-fixtures-docker"
rm -rf "${DOCKER_DIR}"
mkdir -p "${DOCKER_DIR}"
cp "${TARBALL}" "${DOCKER_DIR}/db.tar.gz"

cat > "${DOCKER_DIR}/Dockerfile" << 'EOF'
FROM scratch
LABEL org.opencontainers.image.source=https://github.com/madara-alliance/madara
LABEL org.opencontainers.image.description="Database fixture for migration testing"
COPY db.tar.gz /db.tar.gz
EOF

# Build and push Docker image (multi-platform for CI compatibility)
echo "🐳 Building multi-platform Docker image..."

# Create buildx builder if it doesn't exist
docker buildx create --name multiarch --use 2>/dev/null || docker buildx use multiarch

# Build and push for both arm64 (Mac) and amd64 (CI/Linux)
docker buildx build \
    --platform linux/amd64,linux/arm64 \
    --tag "${IMAGE}" \
    --push \
    "${DOCKER_DIR}"

# Make package public (requires GITHUB_TOKEN with admin:packages or repo scope)
if [ -n "${GITHUB_TOKEN}" ]; then
    echo "🔓 Making package public..."
    PACKAGE_NAME="db-fixture"
    curl -sf -X PATCH \
        -H "Authorization: Bearer ${GITHUB_TOKEN}" \
        -H "Accept: application/vnd.github.v3+json" \
        "https://api.github.com/orgs/madara-alliance/packages/container/${PACKAGE_NAME}" \
        -d '{"visibility":"public"}' \
        && echo "✅ Package is now public" \
        || echo "⚠️  Could not set visibility (you may need to do this manually in GitHub UI)"
else
    echo "⚠️  GITHUB_TOKEN not set - package visibility unchanged"
    echo "   To make public, go to: https://github.com/orgs/madara-alliance/packages"
fi

# Cleanup
rm -rf "${DOCKER_DIR}" "${TARBALL}"

echo ""
echo "============================================"
echo "  ✅ Done!"
echo "============================================"
echo "  Image: ${IMAGE}"
echo "============================================"

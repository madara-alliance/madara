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
#   echo $GITHUB_TOKEN | docker login ghcr.io -u USERNAME --password-stdin
#
# The script will:
#   1. Build madara
#   2. Sync specified blocks from Sepolia
#   3. Package the DB as a Docker image
#   4. Push to ghcr.io/madara-alliance/db-fixtures:v{VERSION}

set -e

VERSION="${1:-8}"
BLOCKS="${2:-50}"
DB_PATH="/tmp/madara-base-db-v${VERSION}"
IMAGE="ghcr.io/madara-alliance/db-fixtures:v${VERSION}"

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

# Clean up any existing DB
rm -rf "${DB_PATH}"
mkdir -p "${DB_PATH}"

# Build madara
echo "🔨 Building madara..."
cd "$(dirname "$0")/../madara"
cargo build --release -p madara

# Sync blocks
echo "🔄 Syncing ${BLOCKS} blocks from Sepolia..."
timeout 900 ./target/release/madara \
    --name base-db-creator \
    --base-path "${DB_PATH}" \
    --network sepolia \
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
COPY db.tar.gz /db.tar.gz
EOF

# Build and push Docker image
echo "🐳 Building Docker image..."
docker build -t "${IMAGE}" "${DOCKER_DIR}"

echo "🚀 Pushing to ghcr.io..."
docker push "${IMAGE}"

# Cleanup
rm -rf "${DOCKER_DIR}" "${TARBALL}"

echo ""
echo "============================================"
echo "  ✅ Done!"
echo "============================================"
echo "  Image: ${IMAGE}"
echo "============================================"

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
#   - GitHub CLI (gh) installed and authenticated
#   - Rust toolchain installed
#
# The script will:
#   1. Build madara
#   2. Sync specified blocks from Sepolia
#   3. Package the DB as a tarball
#   4. Upload to GitHub releases

set -e

VERSION="${1:-8}"
BLOCKS="${2:-50}"
DB_PATH="/tmp/madara-base-db-v${VERSION}"
TARBALL="base-db-v${VERSION}-sepolia.tar.gz"
RELEASE_TAG="db-fixtures-v${VERSION}"

echo "============================================"
echo "  Create Base DB Fixture"
echo "============================================"
echo "  Version: ${VERSION}"
echo "  Blocks:  ${BLOCKS}"
echo "  Path:    ${DB_PATH}"
echo "============================================"
echo ""

# Check prerequisites
if ! command -v gh &> /dev/null; then
    echo "❌ GitHub CLI (gh) not found. Install it first:"
    echo "   brew install gh"
    exit 1
fi

if ! gh auth status &> /dev/null; then
    echo "❌ Not authenticated with GitHub. Run:"
    echo "   gh auth login"
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

# Package
echo "📦 Packaging..."
cd "${DB_PATH}"
tar -czf "/tmp/${TARBALL}" .
ls -lh "/tmp/${TARBALL}"

# Upload to GitHub releases
echo "🚀 Uploading to GitHub releases..."
cd "$(dirname "$0")/.."

# Create release if it doesn't exist
if ! gh release view "${RELEASE_TAG}" &> /dev/null; then
    echo "Creating release ${RELEASE_TAG}..."
    gh release create "${RELEASE_TAG}" \
        --title "DB Fixtures v${VERSION}" \
        --notes "Base DB fixture for migration testing

**Version:** ${VERSION}
**Blocks:** ${BLOCKS}
**Network:** Sepolia

Used by migration tests to validate database migrations."
fi

# Upload asset (overwrite if exists)
gh release upload "${RELEASE_TAG}" "/tmp/${TARBALL}" --clobber

echo ""
echo "============================================"
echo "  ✅ Done!"
echo "============================================"
echo "  Release: ${RELEASE_TAG}"
echo "  Asset:   ${TARBALL}"
echo "  URL:     https://github.com/$(gh repo view --json nameWithOwner -q .nameWithOwner)/releases/tag/${RELEASE_TAG}"
echo "============================================"


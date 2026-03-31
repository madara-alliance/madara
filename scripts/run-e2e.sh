#!/usr/bin/env bash
# Run E2E tests locally, matching the CI workflow exactly.
# Works on both Linux and macOS.
set -euo pipefail

CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}"
AWS_REGION="${AWS_REGION:-us-east-1}"
PATHFINDER_VERSION="v0.14.1-alpha.3"

# ── Colors ──────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
DIM='\033[2;3;37m'
NC='\033[0m'

info()  { echo -e "${DIM}$*${NC}"; }
pass()  { echo -e "${GREEN}✅ $*${NC}"; }
warn()  { echo -e "${YELLOW}⚠️  $*${NC}"; }
fail()  { echo -e "${RED}❌ $*${NC}"; exit 1; }

# ── Step 1: Check dependencies ──────────────────────────────────────
info "Checking dependencies..."

command -v docker &>/dev/null || fail "Docker is not installed"
docker info &>/dev/null 2>&1  || fail "Docker daemon is not running"
command -v anvil  &>/dev/null || fail "Anvil is not installed (install Foundry)"
command -v forge  &>/dev/null || fail "Forge is not installed (install Foundry)"
command -v cargo  &>/dev/null || fail "Cargo is not installed"
pass "All dependencies found"

# ── Step 2: Check .env.e2e ──────────────────────────────────────────
info "Checking .env.e2e..."

if [ ! -f .env.e2e ]; then
    warn ".env.e2e not found — create it with MADARA_ORCHESTRATOR_ATLANTIC_API_KEY"
fi

# Ensure CARGO_TARGET_DIR is set in .env.e2e
if [ -f .env.e2e ]; then
    if grep -q "^CARGO_TARGET_DIR=" .env.e2e; then
        sed -i.bak "s|^CARGO_TARGET_DIR=.*|CARGO_TARGET_DIR=${CARGO_TARGET_DIR}|" .env.e2e && rm -f .env.e2e.bak
    else
        echo "CARGO_TARGET_DIR=${CARGO_TARGET_DIR}" >> .env.e2e
    fi
fi

# ── Step 3: Pull Docker images ──────────────────────────────────────
info "Pulling Docker images..."

docker pull localstack/localstack@sha256:763947722c6c8d33d5fbf7e8d52b4bddec5be35274a0998fdc6176d733375314 2>/dev/null || true
docker pull mongo:latest 2>/dev/null || true
pass "Docker images ready"

# ── Step 4: Build binaries ──────────────────────────────────────────
info "Building binaries (this may take a while)..."
mkdir -p "${CARGO_TARGET_DIR}/release"

info "Building Madara..."
CARGO_TARGET_DIR="${CARGO_TARGET_DIR}" cargo build --manifest-path madara/Cargo.toml --bin madara --release

info "Building Orchestrator..."
CARGO_TARGET_DIR="${CARGO_TARGET_DIR}" cargo build --manifest-path orchestrator/Cargo.toml --bin orchestrator --release

info "Building Bootstrapper V2..."
CARGO_TARGET_DIR="${CARGO_TARGET_DIR}" cargo build --manifest-path bootstrapper-v2/Cargo.toml --bin bootstrapper-v2 --release

info "Building E2E test package..."
CARGO_TARGET_DIR="${CARGO_TARGET_DIR}" cargo build -p e2e

pass "All binaries built"

# ── Step 5: Download Pathfinder ─────────────────────────────────────
info "Downloading Pathfinder..."

if [ ! -f "${CARGO_TARGET_DIR}/release/pathfinder" ]; then
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    if [ "$OS" = "Darwin" ] && [ "$ARCH" = "arm64" ]; then
        PF_URL="https://github.com/karnotxyz/pathfinder/releases/download/${PATHFINDER_VERSION}/pathfinder-aarch64-apple-darwin.tar.gz"
    elif [ "$OS" = "Linux" ] && [ "$ARCH" = "x86_64" ]; then
        PF_URL="https://github.com/karnotxyz/pathfinder/releases/download/${PATHFINDER_VERSION}/pathfinder-x86_64-unknown-linux-gnu.tar.gz"
    else
        fail "Unsupported platform: ${OS}/${ARCH}"
    fi

    curl -L -o pathfinder.tar.gz "$PF_URL"
    tar -xf pathfinder.tar.gz -C "${CARGO_TARGET_DIR}/release/"
    rm pathfinder.tar.gz
    pass "Pathfinder downloaded"
else
    info "Pathfinder binary already exists, skipping download"
fi

# ── Step 6: Make binaries executable ────────────────────────────────
chmod +x "${CARGO_TARGET_DIR}/release/madara"
chmod +x "${CARGO_TARGET_DIR}/release/bootstrapper-v2"
chmod +x "${CARGO_TARGET_DIR}/release/pathfinder"
chmod +x "${CARGO_TARGET_DIR}/release/orchestrator"
chmod +x test_utils/scripts/deploy_dummy_verifier.sh
pass "Binaries are executable"

# ── Step 7: Run E2E test ────────────────────────────────────────────
info "Running E2E bridge tests..."

AWS_REGION="${AWS_REGION}" \
CARGO_TARGET_DIR="${CARGO_TARGET_DIR}" \
RUST_LOG=info \
cargo test \
    --package e2e test_bridge_deposit_and_withdraw \
    -- --test-threads=10 --nocapture

pass "E2E bridge tests completed!"

# ── Step 8: Cleanup ────────────────────────────────────────────────
info "Cleaning up e2e_data directory..."
rm -rf e2e_data
pass "Cleanup done"

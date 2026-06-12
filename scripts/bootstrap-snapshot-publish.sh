#!/usr/bin/env bash
#
# Create a canonical Madara bootstrap snapshot package from an already-synced
# database. This script does not sync the node or stop a running service; the
# source base path must be readable by Madara and not locked by another process.

set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/bootstrap-snapshot-publish.sh --base-path PATH --output-dir PATH [options]

Options:
  --madara-bin PATH        Madara binary to run (default: target/release/madara)
  --network NAME           Chain network to pass to Madara (default: mainnet)
  --base-path PATH         Existing synced Madara base path
  --output-dir PATH        Directory where canonical snapshot files are written
  --name-prefix PREFIX     Archive filename prefix (default: madara)
  --extra-madara-arg ARG   Extra single argument to pass to Madara. Repeatable.
  -h, --help               Show this help

Environment defaults:
  MADARA_BIN
  MADARA_BOOTSTRAP_SNAPSHOT_NETWORK
  MADARA_BOOTSTRAP_SNAPSHOT_BASE_PATH
  MADARA_BOOTSTRAP_SNAPSHOT_OUTPUT_DIR
  MADARA_BOOTSTRAP_SNAPSHOT_NAME_PREFIX

The script writes:
  <output-dir>/<prefix>-<network>-<block padded to 9 digits>.tar.gz
  <output-dir>/<prefix>-<network>-<block padded to 9 digits>.tar.gz.manifest.json
  <output-dir>/latest.txt
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required"
}

realpath_dir() {
  local path="$1"
  [ -d "$path" ] || die "$path is not a directory"
  (cd "$path" && pwd -P)
}

realpath_file() {
  local path="$1"
  [ -f "$path" ] || die "$path is not a file"
  local dir
  dir="$(cd "$(dirname "$path")" && pwd -P)"
  printf '%s/%s\n' "$dir" "$(basename "$path")"
}

file_size() {
  local path="$1"
  if stat -c '%s' "$path" >/dev/null 2>&1; then
    stat -c '%s' "$path"
  else
    stat -f '%z' "$path"
  fi
}

sha256_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{ print $1 }'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{ print $1 }'
  else
    die "sha256sum or shasum is required"
  fi
}

lowercase() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

emit_output() {
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    printf '%s=%s\n' "$1" "$2" >> "$GITHUB_OUTPUT"
  fi
}

madara_bin="${MADARA_BIN:-target/release/madara}"
network="${MADARA_BOOTSTRAP_SNAPSHOT_NETWORK:-mainnet}"
base_path="${MADARA_BOOTSTRAP_SNAPSHOT_BASE_PATH:-}"
output_dir="${MADARA_BOOTSTRAP_SNAPSHOT_OUTPUT_DIR:-}"
name_prefix="${MADARA_BOOTSTRAP_SNAPSHOT_NAME_PREFIX:-madara}"
extra_madara_args=()

while [ "$#" -gt 0 ]; do
  case "$1" in
    --madara-bin)
      [ "$#" -ge 2 ] || die "--madara-bin requires a value"
      madara_bin="$2"
      shift 2
      ;;
    --network)
      [ "$#" -ge 2 ] || die "--network requires a value"
      network="$2"
      shift 2
      ;;
    --base-path)
      [ "$#" -ge 2 ] || die "--base-path requires a value"
      base_path="$2"
      shift 2
      ;;
    --output-dir)
      [ "$#" -ge 2 ] || die "--output-dir requires a value"
      output_dir="$2"
      shift 2
      ;;
    --name-prefix)
      [ "$#" -ge 2 ] || die "--name-prefix requires a value"
      name_prefix="$2"
      shift 2
      ;;
    --extra-madara-arg)
      [ "$#" -ge 2 ] || die "--extra-madara-arg requires a value"
      extra_madara_args+=("$2")
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

[ -n "$base_path" ] || die "--base-path is required"
[ -n "$output_dir" ] || die "--output-dir is required"

require_cmd jq
require_cmd tar
require_cmd awk
require_cmd grep

madara_bin="$(realpath_file "$madara_bin")"
[ -x "$madara_bin" ] || die "$madara_bin is not executable"

base_path="$(realpath_dir "$base_path")"
[ -f "$base_path/.db-version" ] || die "$base_path does not look like a Madara base path: missing .db-version"

mkdir -p "$output_dir"
output_dir="$(realpath_dir "$output_dir")"

case "$output_dir/" in
  "$base_path/"*) die "--output-dir must be outside --base-path" ;;
esac

tmp_dir="$(mktemp -d "$output_dir/.bootstrap-snapshot.XXXXXX")"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

raw_archive="$tmp_dir/snapshot.tar.gz"
raw_manifest="$raw_archive.manifest.json"

madara_cmd=(
  "$madara_bin"
  --full
  --network "$network"
  --base-path "$base_path"
  --no-l1-sync
)
if [ "${#extra_madara_args[@]}" -gt 0 ]; then
  madara_cmd+=("${extra_madara_args[@]}")
fi
madara_cmd+=(--create-bootstrap-snapshot "$raw_archive")

echo "Creating bootstrap snapshot from $base_path"
"${madara_cmd[@]}"

[ -f "$raw_archive" ] || die "Madara did not create $raw_archive"
[ -f "$raw_manifest" ] || die "Madara did not create $raw_manifest"

format_version="$(jq -er '.format_version' "$raw_manifest")"
[ "$format_version" = "1" ] || die "unsupported manifest format_version=$format_version"

block_number="$(jq -er '.block_number | numbers' "$raw_manifest")"
[[ "$block_number" =~ ^[0-9]+$ ]] || die "invalid manifest block_number=$block_number"

chain_id="$(jq -er '.chain_id' "$raw_manifest")"
archive_hash="$(jq -er '.archive_sha256' "$raw_manifest")"
[[ "$archive_hash" =~ ^[0-9a-fA-F]{64}$ ]] || die "invalid manifest archive_sha256=$archive_hash"

expected_size="$(jq -r '.archive_size_bytes // empty' "$raw_manifest")"
if [ -n "$expected_size" ]; then
  actual_size="$(file_size "$raw_archive")"
  [ "$actual_size" = "$expected_size" ] || die "archive size mismatch: expected $expected_size, got $actual_size"
fi

actual_hash="$(sha256_file "$raw_archive")"
if [ "$(lowercase "$actual_hash")" != "$(lowercase "$archive_hash")" ]; then
  die "archive sha256 mismatch: expected $archive_hash, got $actual_hash"
fi

archive_listing="$tmp_dir/archive.list"
tar -tzf "$raw_archive" > "$archive_listing"
grep -Eq '^(\./)?db(/|$)' "$archive_listing" || die "archive is missing db/"
grep -Eq '^(\./)?\.db-version$' "$archive_listing" || die "archive is missing .db-version"

printf -v padded_block '%09d' "$block_number"
archive_name="${name_prefix}-${network}-${padded_block}.tar.gz"
final_archive="$output_dir/$archive_name"
final_manifest="$final_archive.manifest.json"
latest_path="$output_dir/latest.txt"

[ ! -e "$final_archive" ] || die "$final_archive already exists"
[ ! -e "$final_manifest" ] || die "$final_manifest already exists"

mv "$raw_archive" "$final_archive"
mv "$raw_manifest" "$final_manifest"
printf '%s\n' "$archive_name" > "$tmp_dir/latest.txt"
mv "$tmp_dir/latest.txt" "$latest_path"

emit_output archive_path "$final_archive"
emit_output archive_name "$archive_name"
emit_output manifest_path "$final_manifest"
emit_output latest_path "$latest_path"
emit_output block_number "$block_number"
emit_output chain_id "$chain_id"

echo "Bootstrap snapshot ready:"
echo "  archive:  $final_archive"
echo "  manifest: $final_manifest"
echo "  latest:   $latest_path"
echo "  block:    $block_number"
echo "  chain id: $chain_id"

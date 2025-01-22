#!/bin/bash

#
# Database version management script
#
# This script updates the database version tracking file when schema changes occur.
# It's typically called by CI when a PR with the 'db-migration' label is merged.
#
# Requirements: yq (https://github.com/mikefarah/yq/)
#
# Usage:
#   ./update-db-version.sh PR_NUMBER
#
# Arguments:
#   PR_NUMBER - The pull request number that introduced the schema changes
#
# File format (.db-versions.yml):
#   current_version: 41
#   versions:
#     - version: 41
#       pr: 120
#
# Environment:
#   No specific environment variables required
#
# Exit codes:
#   0 - Success
#   1 - Usage error
#   1 - Missing dependencies
#   1 - File read/write error
#   1 - Version parsing error
#   1 - PR already exists
#
# Example:
#   ./update-db-version.sh 123
#   Successfully updated DB version from 41 to 42 (PR #123)

set -euo pipefail

FILE=".db-versions.yml"

[ $# -eq 1 ] || { echo "Usage: $0 PR_NUMBER" >&2; exit 1; }
PR_NUMBER="$1"

command -v yq >/dev/null 2>&1 || { echo "Error: yq is required but not installed" >&2; exit 1; }

[ -f "$FILE" ] || { echo "Error: $FILE not found" >&2; exit 1; }

# Check duplicate PR
yq -e ".versions[] | select(.pr == $PR_NUMBER)" "$FILE" >/dev/null 2>&1 && {
  echo "Error: PR #${PR_NUMBER} already exists in version history" >&2
  exit 1
}


# Get and validate current version
CURRENT_VERSION=$(yq '.current_version' "$FILE")
[[ "$CURRENT_VERSION" =~ ^[0-9]+$ ]] || {
  echo "Error: Invalid current_version in $FILE" >&2
  exit 1
}

# Increment the version
NEW_VERSION=$((CURRENT_VERSION + 1))

# Update version and append to history
yq e ".current_version = $NEW_VERSION |
  .versions = [{\"version\": $NEW_VERSION, \"pr\": $PR_NUMBER}] + .versions" -i "$FILE"

echo "Successfully updated DB version to ${NEW_VERSION} (PR #${PR_NUMBER})"
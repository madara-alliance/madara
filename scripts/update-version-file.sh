#!/bin/bash

#
# Version file management script
#
# This script updates the version tracking file to mark changes in external
# dependencies, for example database schema changes or cairo artifact changes.
#
# It's typically called by CI when a PR with a specific label, such as
# 'db-migration', is merged.
#
# Requirements: yq (https://github.com/mikefarah/yq/)
#
# Usage:
#   ./update-version-file.sh PR_NUMBER FILE
#
# Arguments:
#   PR_NUMBER - The pull request number that introduced the schema changes
#   FILE      - Path to the version file to save
#
# File format:
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
#   ./update-version-file.sh 123 .db-versions.yml
#   Successfully updated '.db-versions.yml' from 41 to 42 (PR #123)

set -euo pipefail


[ $# -eq 2 ] || { echo "Usage: $0 PR_NUMBER FILE" >&2; exit 1; }
PR_NUMBER="$1"
FILE="$2"

command -v yq >/dev/null 2>&1 || { echo "Error: yq is required but not installed" >&2; exit 1; }

[ -f "$FILE" ] || { echo "Creating $FILE"; touch "$FILE" }

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

echo "Successfully updated ${FILE} to ${NEW_VERSION} (PR #${PR_NUMBER})"

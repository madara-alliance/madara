#!/bin/sh

#
# Database version management script
#
# This script updates the database version tracking file when schema changes occur.
# It's typically called by CI when a PR with the 'bump_db' label is merged.
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
#   1 - File read/write error
#   1 - Version parsing error
#   1 - PR already exists
#
# Example:
#   ./update-db-version.sh 123
#   Successfully updated DB version from 41 to 42 (PR #123)

FILE=".db-versions.yml"

if [ $# -eq 0 ]; then
    echo "Usage: $0 PR_NUMBER" 
    exit 1
fi
set -euo pipefail

PR_NUMBER="$1"

# Check if the file exists
if [ ! -f "$FILE" ]; then
    echo "Error: $FILE not found"
    exit 1
fi

# Check if PR already exists in the versions
if yq -e ".versions[] | select(.pr == $PR_NUMBER)" "$FILE" > /dev/null 2>&1; then
    echo "Error: PR #${PR_NUMBER} already exists in version history"
    exit 1
fi

# Read and validate the current version
CURRENT_VERSION=$(yq '.current_version' "$FILE")
if ! [[ "$CURRENT_VERSION" =~ ^[0-9]+$ ]]; then
    echo "Error: Failed to read current_version from $FILE"
    exit 1
fi

# Increment the version
NEW_VERSION=$((CURRENT_VERSION + 1))

# Update the file
yq -i ".current_version = $NEW_VERSION" "$FILE"
yq -i ".versions = [{ \"version\": $NEW_VERSION, \"pr\": $PR_NUMBER }] + .versions" "$FILE"

echo "Successfully updated DB version to ${NEW_VERSION} (PR #${PR_NUMBER})"
#!/bin/bash

# Script to set up AWS resources for Madara Orchestrator
# Usage: ./run_setup_resources.sh <prefix>
# Example: ./run_setup_resources.sh dev

# Check if prefix argument is provided
if [ -z "$1" ]; then
  echo "Error: Prefix argument is required"
  echo "Usage: ./run_setup_resources.sh <prefix>"
  echo "Example: ./run_setup_resources.sh dev"
  exit 1
fi

PREFIX=$1
REGION=${2:-us-west-1}  # Default to us-west-1 if not provided
PROFILE=${3:-default}   # Default to 'default' AWS profile if not provided

echo "Setting up resources with prefix: $PREFIX"
echo "Using AWS region: $REGION"
echo "Using AWS profile: $PROFILE"

# Export AWS profile for the AWS SDK
export AWS_PROFILE=$PROFILE

# Navigate to the orchestrator directory
cd $(dirname "$0")/crates/orchestrator

# Run the setup resources example with the provided arguments
cargo run --example setup_resources -- \
  --prefix "$PREFIX" \
  --region "$REGION" \
  --bucket-name "bucket" \
  --queue-suffix "queue" \
  --sqs-base-url "https://sqs.$REGION.amazonaws.com/"

echo "Resources setup completed."

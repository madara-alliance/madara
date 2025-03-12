#!/bin/bash
# Run this script with: ./run_resources_setup.sh <prefix>
# Example: ./run_resources_setup.sh eenthiran

# Exit on any error
set -e

# Validate arguments
if [ -z "$1" ]; then
  echo "Error: Missing prefix argument"
  echo "Usage: $0 <prefix>"
  echo "Example: $0 eenthiran"
  exit 1
fi

# Export required environment variables
export MADARA_ORCHESTRATOR_PREFIX="$1"
export MADARA_ORCHESTRATOR_AWS_S3_BUCKET_NAME="bucket"
export MADARA_ORCHESTRATOR_SQS_BASE_QUEUE_URL="https://sqs.us-west-1.amazonaws.com/"
export MADARA_ORCHESTRATOR_SQS_SUFFIX="queue"
export AWS_REGION="us-west-1"
export AWS_PROFILE="default-mfa"

echo "Setting up resources with prefix: $MADARA_ORCHESTRATOR_PREFIX"
echo "AWS Region: $AWS_REGION"
echo "AWS Profile: $AWS_PROFILE"

# Run the setup resources example
cd crates/orchestrator
cargo run --example setup_resources

echo "Resources setup completed!"

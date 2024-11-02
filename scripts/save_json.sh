#!/bin/bash

ENV_FILE="$(pwd)/.makefile.json"
key="$1"
value="$2"

# Create the environment file if it doesn't exist
if [ ! -f "$ENV_FILE" ]; then
  echo '{}' > "$ENV_FILE"
fi

# Use jq to update the JSON file with the new key-value pair
jq --arg key "$key" --arg value "$value" \
   '.[$key] = $value' "$ENV_FILE" > "$ENV_FILE.tmp" && \
mv "$ENV_FILE.tmp" "$ENV_FILE"
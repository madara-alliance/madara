#!/bin/bash

# Check if required arguments are provided
if [ "$#" -ne 3 ]; then
    echo "Usage: $0 <json_file_path> <key> <value>"
    exit 1
fi

json_file="$1"
key="$2"
value="$3"

# Create directory structure if it doesn't exist
directory=$(dirname "$json_file")
mkdir -p "$directory"

# If file doesn't exist, create an empty JSON object
if [ ! -f "$json_file" ]; then
    echo "{}" > "$json_file"
fi

# Check if the file is valid JSON
if ! jq empty "$json_file" 2>/dev/null; then
    echo "Error: File exists but is not valid JSON"
    exit 1
fi

# Update or add the key-value pair
# The value is treated as a string. If you need to preserve type,
# remove the quotes around "$value" in the jq command
jq --arg k "$key" --arg v "$value" '.[$k] = $v' "$json_file" > "$json_file.tmp"

# Check if jq command was successful
if [ $? -eq 0 ]; then
    mv "$json_file.tmp" "$json_file"
    echo "Successfully updated $json_file"
    echo "Current content:"
    cat "$json_file"
else
    rm -f "$json_file.tmp"
    echo "Error: Failed to update JSON file"
    exit 1
fi
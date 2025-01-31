#!/bin/bash

# Function to recursively scan files for TODO and FIXME comments
scan_todos() {
    local dir=$1
    local ext=$2
    local max_depth=$3

    echo "Scanning directory: $dir"

    # Validate directory
    if [ ! -d "$dir" ]; then
        echo "Error: Directory $dir does not exist or is inaccessible."
        exit 1
    fi

    # Build the find command based on whether an extension is provided
    local find_cmd="find \"$dir\" -type f"
    if [ -n "$ext" ]; then
        find_cmd+=" -name \"*.$ext\""
    fi
    if [ -n "$max_depth" ]; then
        find_cmd+=" -maxdepth $max_depth"
    fi

    # Execute the find command and process the files
    eval $find_cmd | while read -r file; do
        # Search for TODO/FIXME and group results by file
        matches=$(grep -n -E "TODO|FIXME" "$file")
        if [ -n "$matches" ]; then
            echo -e "\nFile: $file"
            echo "$matches" | sed 's/^/  /'  # Indent for better readability
        fi
    done
}

# Check if a directory argument is provided
if [ $# -lt 1 ]; then
    echo "Usage: $0 <directory> [extension] [max-depth]"
    echo "Example: $0 ./crates rs 3"
    echo "Example (no extension): $0 ./crates \"\""
    exit 1
fi

# Set default values for optional arguments
directory=$1
extension=$2      # If empty, all files will be scanned
max_depth=${3:-}  # No depth limit if not provided

# Call the function
scan_todos "$directory" "$extension" "$max_depth"
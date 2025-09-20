#!/bin/bash

# Build Madara Docker image for AMD64 architecture

# Build for AMD64 (standard x86_64)
echo "Building Madara Docker image for AMD64..."
docker build --platform linux/amd64 -t madara:amd64 -t madara:latest .

# Optional: Build with buildx for multi-platform support
# Requires Docker buildx to be set up
# docker buildx build --platform linux/amd64 -t madara:amd64 --load .

echo "Build complete!"
echo "To run the container: docker run --rm madara:amd64"
#!/bin/bash

# Docker image details
DOCKER_REPO="itsparser/madara"
TAG="0.0.1"
PLATFORM="linux/amd64"

echo "Building Madara Docker image..."
echo "Repository: ${DOCKER_REPO}"
echo "Tag: ${TAG}"
echo "Platform: ${PLATFORM}"

# Build the Docker image
echo "Step 1: Building Docker image..."
docker build --platform ${PLATFORM} -t ${DOCKER_REPO}:${TAG} -t ${DOCKER_REPO}:latest .

if [ $? -ne 0 ]; then
    echo "❌ Docker build failed!"
    exit 1
fi

echo "✅ Docker image built successfully!"

# Login to Docker Hub (if not already logged in)
echo "Step 2: Logging in to Docker Hub..."
echo "Please ensure you're logged in to Docker Hub as 'itsparser'"
docker login

if [ $? -ne 0 ]; then
    echo "❌ Docker login failed!"
    exit 1
fi

# Push the image
echo "Step 3: Pushing image to Docker Hub..."
docker push ${DOCKER_REPO}:${TAG}
docker push ${DOCKER_REPO}:latest

if [ $? -ne 0 ]; then
    echo "❌ Docker push failed!"
    exit 1
fi

echo "✅ Successfully pushed ${DOCKER_REPO}:${TAG} and ${DOCKER_REPO}:latest to Docker Hub!"
echo "Image is now available at: https://hub.docker.com/r/${DOCKER_REPO}"
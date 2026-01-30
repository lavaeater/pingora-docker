#!/bin/bash
set -e

REPO_FULL_NAME="$1"
REF="$2"
REPOS_DIR="/repos"
STACK_FILE="/compose/docker-stack.yml"
STACK_NAME="pingora"

# Registry address - should be set via environment variable
REGISTRY="${REGISTRY:-localhost:5000}"

# Only process tag refs (refs/tags/v0.1.0 -> v0.1.0)
if [[ ! "$REF" =~ ^refs/tags/ ]]; then
    echo "Ignoring non-tag ref: $REF"
    exit 0
fi

TAG="${REF#refs/tags/}"

echo "=== Swarm Webhook triggered ==="
echo "Repository: $REPO_FULL_NAME"
echo "Ref: $REF"
echo "Tag: $TAG"
echo "Registry: $REGISTRY"

# Load service mappings from config
CONFIG_FILE="/config/services.json"

if [ ! -f "$CONFIG_FILE" ]; then
    echo "ERROR: Config file not found: $CONFIG_FILE"
    exit 1
fi

# Find matching service using jq
SERVICE_NAME=$(jq -r --arg repo "$REPO_FULL_NAME" '
    to_entries[] | 
    select(.value.repo == $repo) | 
    .key
' "$CONFIG_FILE")

if [ -z "$SERVICE_NAME" ] || [ "$SERVICE_NAME" = "null" ]; then
    echo "No matching service found for $REPO_FULL_NAME"
    exit 0
fi

echo "Matched service: $SERVICE_NAME"

# Get repo URL and build context
REPO_URL=$(jq -r --arg service "$SERVICE_NAME" '.[$service].url' "$CONFIG_FILE")
BUILD_CONTEXT=$(jq -r --arg service "$SERVICE_NAME" '.[$service].build_context // "."' "$CONFIG_FILE")
DOCKERFILE=$(jq -r --arg service "$SERVICE_NAME" '.[$service].dockerfile // "Dockerfile"' "$CONFIG_FILE")

REPO_DIR="$REPOS_DIR/$SERVICE_NAME"

echo "Repo URL: $REPO_URL"
echo "Local path: $REPO_DIR"
echo "Build context: $BUILD_CONTEXT"
echo "Dockerfile: $DOCKERFILE"

# Clone or pull the repository and checkout the tag
if [ -d "$REPO_DIR/.git" ]; then
    echo "Fetching and checking out tag $TAG..."
    cd "$REPO_DIR"
    git fetch origin --tags
    git checkout "$TAG"
else
    echo "Cloning repository..."
    mkdir -p "$REPO_DIR"
    git clone "$REPO_URL" "$REPO_DIR"
    cd "$REPO_DIR"
    git checkout "$TAG"
fi

# Build the image
IMAGE_NAME="$REGISTRY/$SERVICE_NAME:$TAG"
IMAGE_LATEST="$REGISTRY/$SERVICE_NAME:latest"

echo "Building image: $IMAGE_NAME"
docker build -t "$IMAGE_NAME" -t "$IMAGE_LATEST" -f "$DOCKERFILE" "$BUILD_CONTEXT"

# Push to registry
echo "Pushing to registry..."
docker push "$IMAGE_NAME"
docker push "$IMAGE_LATEST"

# Update the swarm service
SWARM_SERVICE="${STACK_NAME}_${SERVICE_NAME}"
echo "Updating swarm service: $SWARM_SERVICE"

# Check if service exists
if docker service inspect "$SWARM_SERVICE" > /dev/null 2>&1; then
    # Update existing service with new image
    docker service update --image "$IMAGE_LATEST" "$SWARM_SERVICE"
else
    echo "Service $SWARM_SERVICE not found. Deploying full stack..."
    docker stack deploy -c "$STACK_FILE" "$STACK_NAME"
fi

echo "=== Swarm rebuild complete for $SERVICE_NAME ==="

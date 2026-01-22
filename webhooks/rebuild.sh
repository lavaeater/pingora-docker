#!/bin/bash
set -e

REPO_FULL_NAME="$1"
REF="$2"
REPOS_DIR="/repos"
COMPOSE_FILE="/compose/docker-compose.yml"
PROJECT_NAME="pingora-docker"

# Only process tag refs (refs/tags/v0.1.0 -> v0.1.0)
if [[ ! "$REF" =~ ^refs/tags/ ]]; then
    echo "Ignoring non-tag ref: $REF"
    exit 0
fi

TAG="${REF#refs/tags/}"

echo "=== Webhook triggered ==="
echo "Repository: $REPO_FULL_NAME"
echo "Ref: $REF"
echo "Tag: $TAG"

# Load service mappings from config
CONFIG_FILE="/config/services.json"

if [ ! -f "$CONFIG_FILE" ]; then
    echo "ERROR: Config file not found: $CONFIG_FILE"
    exit 1
fi

# Find matching service using jq (match by repo only, tags apply to all branches)
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

# Get repo URL
REPO_URL=$(jq -r --arg service "$SERVICE_NAME" '.[$service].url' "$CONFIG_FILE")
REPO_DIR="$REPOS_DIR/$SERVICE_NAME"

echo "Repo URL: $REPO_URL"
echo "Local path: $REPO_DIR"

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

# Rebuild and restart the container
echo "Rebuilding container: $SERVICE_NAME"
#docker compose -f "$COMPOSE_FILE" stop "$SERVICE_NAME" || true
#docker compose -f "$COMPOSE_FILE" rm -f "$SERVICE_NAME" || true
docker compose -p "$PROJECT_NAME" -f "$COMPOSE_FILE" up --no-deps --build -d "$SERVICE_NAME"

echo "=== Rebuild complete for $SERVICE_NAME ==="

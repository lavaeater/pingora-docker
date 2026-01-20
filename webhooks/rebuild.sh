#!/bin/bash
set -e

REPO_FULL_NAME="$1"
REF="$2"
REPOS_DIR="/repos"
COMPOSE_FILE="/compose/docker-compose.yml"
PROJECT_NAME="pingora-docker"

# Extract branch name from ref (refs/heads/main -> main)
BRANCH="${REF#refs/heads/}"

echo "=== Webhook triggered ==="
echo "Repository: $REPO_FULL_NAME"
echo "Ref: $REF"
echo "Branch: $BRANCH"

# Load service mappings from config
CONFIG_FILE="/config/services.json"

if [ ! -f "$CONFIG_FILE" ]; then
    echo "ERROR: Config file not found: $CONFIG_FILE"
    exit 1
fi

# Find matching service using jq
SERVICE_NAME=$(jq -r --arg repo "$REPO_FULL_NAME" --arg branch "$BRANCH" '
    to_entries[] | 
    select(.value.repo == $repo and .value.branch == $branch) | 
    .key
' "$CONFIG_FILE")

if [ -z "$SERVICE_NAME" ] || [ "$SERVICE_NAME" = "null" ]; then
    echo "No matching service found for $REPO_FULL_NAME on branch $BRANCH"
    exit 0
fi

echo "Matched service: $SERVICE_NAME"

# Get repo URL
REPO_URL=$(jq -r --arg service "$SERVICE_NAME" '.[$service].url' "$CONFIG_FILE")
REPO_DIR="$REPOS_DIR/$SERVICE_NAME"

echo "Repo URL: $REPO_URL"
echo "Local path: $REPO_DIR"

# Clone or pull the repository
if [ -d "$REPO_DIR/.git" ]; then
    echo "Pulling latest changes..."
    cd "$REPO_DIR"
    git fetch origin
    git reset --hard "origin/$BRANCH"
else
    echo "Cloning repository..."
    mkdir -p "$REPO_DIR"
    git clone --branch "$BRANCH" "$REPO_URL" "$REPO_DIR"
fi

# Rebuild and restart the container
echo "Rebuilding container: $SERVICE_NAME"
#docker compose -f "$COMPOSE_FILE" stop "$SERVICE_NAME" || true
#docker compose -f "$COMPOSE_FILE" rm -f "$SERVICE_NAME" || true
docker compose -p "$PROJECT_NAME" -f "$COMPOSE_FILE" up --no-deps --build -d "$SERVICE_NAME"

echo "=== Rebuild complete for $SERVICE_NAME ==="

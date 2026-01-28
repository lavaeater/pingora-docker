#!/bin/bash
set -e

REPOS_DIR="/repos"
CONFIG_FILE="/config/services.json"
COMPOSE_FILE="/compose/docker-compose.yml"
PROJECT_NAME="pingora-docker"

echo "=== Initializing services ==="

if [ ! -f "$CONFIG_FILE" ]; then
    echo "ERROR: Config file not found: $CONFIG_FILE"
    exit 1
fi

# Iterate over all services in the config
for SERVICE_NAME in $(jq -r 'keys[]' "$CONFIG_FILE"); do
    REPO_URL=$(jq -r --arg service "$SERVICE_NAME" '.[$service].url' "$CONFIG_FILE")
    REPO_DIR="$REPOS_DIR/$SERVICE_NAME"
    
    echo "--- Processing: $SERVICE_NAME ---"
    echo "Repo URL: $REPO_URL"
    echo "Local path: $REPO_DIR"
    
    if [ -d "$REPO_DIR/.git" ]; then
        echo "Repository exists, fetching updates..."
        cd "$REPO_DIR"
        git fetch origin --tags
    else
        echo "Cloning repository..."
        mkdir -p "$REPO_DIR"
        git clone "$REPO_URL" "$REPO_DIR"
        cd "$REPO_DIR"
    fi
    
    # Get the latest tag (sorted by version)
    LATEST_TAG=$(git tag -l --sort=-v:refname | head -n 1)
    
    if [ -z "$LATEST_TAG" ]; then
        echo "WARNING: No tags found for $SERVICE_NAME, using default branch"
    else
        echo "Checking out latest tag: $LATEST_TAG"
        git checkout "$LATEST_TAG"
    fi
    
    echo ""
done

echo "=== All repositories initialized ==="

# Optionally build and start services
if [ "$1" = "--build" ]; then
    echo "=== Building and starting services ==="
    for SERVICE_NAME in $(jq -r 'keys[]' "$CONFIG_FILE"); do
        echo "Building: $SERVICE_NAME"
        docker compose -p "$PROJECT_NAME" -f "$COMPOSE_FILE" up --no-deps --build -d "$SERVICE_NAME"
    done
    echo "=== All services started ==="
fi

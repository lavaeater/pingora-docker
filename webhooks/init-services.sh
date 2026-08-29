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

    # Submodules (e.g. civilization's lava_ui_builder): a plain clone/checkout
    # leaves these as empty directories, which breaks any build that needs
    # them. Harmless no-op for repos without submodules.
    git submodule update --init --recursive

    echo ""
done

echo "=== All repositories initialized ==="

# Optionally build and start services
if [ "$1" = "--build" ]; then
    echo "=== Building and starting services ==="
    for SERVICE_NAME in $(jq -r 'keys[]' "$CONFIG_FILE"); do
        # A repo may map to more than one compose service (e.g. civilization
        # -> civilization-server + civilization-web). Falls back to
        # [SERVICE_NAME] for repos that only ever mapped 1:1.
        mapfile -t COMPOSE_SERVICES < <(jq -r --arg service "$SERVICE_NAME" '
            .[$service].compose_services // [$service] | .[]
        ' "$CONFIG_FILE")
        for COMPOSE_SERVICE in "${COMPOSE_SERVICES[@]}"; do
            echo "Building: $COMPOSE_SERVICE"
            docker compose -p "$PROJECT_NAME" -f "$COMPOSE_FILE" up --no-deps --build -d "$COMPOSE_SERVICE"
        done
    done
    echo "=== All services started ==="
fi

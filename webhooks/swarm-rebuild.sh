#!/bin/bash
set -e

REPO_FULL_NAME="$1"
REF="$2"
REPOS_DIR="/repos"
STACK_FILE="/compose/docker-stack.yml"
STACK_NAME="pingora"

# Registry address - should be set via environment variable
REGISTRY="${REGISTRY:-localhost:5000}"

# Report the deploy to GitHub as a commit status (context "deploy/home"), so
# the tagged commit shows a check next to its CI result. No-op until
# GITHUB_STATUS_TOKEN is set and the commit SHA is known.
post_status() {
    local state="$1" description="$2"
    if [ -z "${GITHUB_STATUS_TOKEN:-}" ] || [ -z "${SHA:-}" ]; then
        return 0
    fi
    curl -fsS -o /dev/null -X POST \
        -H "Authorization: Bearer $GITHUB_STATUS_TOKEN" \
        -H "Accept: application/vnd.github+json" \
        "https://api.github.com/repos/$REPO_FULL_NAME/statuses/$SHA" \
        -d "$(jq -n --arg s "$state" --arg d "$description" \
              '{state: $s, description: $d, context: "deploy/home"}')" \
        || echo "WARNING: Failed to post commit status to GitHub"
}

on_error() {
    local exit_code=$?
    post_status failure "Deploy of ${TAG:-unknown} failed (exit $exit_code)"
    exit $exit_code
}
trap on_error ERR

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

# One heavy build at a time on this machine. The lock file lives in the
# bind-mounted repos dir, so it is shared with other deploys *and* with CI
# jobs on the host (ci/ci-run.sh takes the same lock). Held until this script
# exits. Also keeps two deploys of one service off the same checkout.
LOCK_DIR="$REPOS_DIR/.locks"
mkdir -p "$LOCK_DIR"
exec 9>>"$LOCK_DIR/build.lock"
chmod 777 "$LOCK_DIR" 2>/dev/null || true
chmod 666 "$LOCK_DIR/build.lock" 2>/dev/null || true
if ! flock -n 9; then
    echo "Another build holds the lock, waiting..."
    flock 9
fi

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

SHA=$(git rev-parse 'HEAD^{commit}')
post_status pending "Deploying $TAG"

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

post_status success "Deployed $TAG"

echo "=== Swarm rebuild complete for $SERVICE_NAME ==="

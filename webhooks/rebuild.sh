#!/bin/bash

REPO_FULL_NAME="$1"
REF="$2"
REPOS_DIR="/repos"
COMPOSE_FILE="/compose/docker-compose.yml"
PROJECT_NAME="pingora-docker"

send_notification() {
    local subject="$1"
    local body="$2"

    if [ -z "$NOTIFY_EMAIL" ] || [ ! -f /etc/msmtprc ]; then
        return 0
    fi

    printf "To: %s\nFrom: %s\nSubject: %s\n\n%s" \
        "$NOTIFY_EMAIL" "$SMTP_FROM" "$subject" "$body" \
        | msmtp "$NOTIFY_EMAIL" 2>/dev/null || echo "WARNING: Failed to send email notification"
}

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

# Trap errors so we can notify on failure
on_error() {
    local exit_code=$?
    post_status failure "Deploy of ${TAG:-unknown} failed (exit $exit_code)"
    send_notification \
        "[FAIL] Rebuild failed: ${SERVICE_NAME:-unknown}" \
        "Service: ${SERVICE_NAME:-unknown}
Repository: $REPO_FULL_NAME
Tag: ${TAG:-unknown}
Exit code: $exit_code
Time: $(date -u '+%Y-%m-%d %H:%M:%S UTC')"
    exit $exit_code
}
trap on_error ERR
set -e

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

# Submodules (e.g. civilization's lava_ui_builder): a plain clone/checkout
# leaves these as empty directories, which breaks any build that needs them.
# Harmless no-op for repos without submodules.
git submodule update --init --recursive

SHA=$(git rev-parse 'HEAD^{commit}')
post_status pending "Deploying $TAG"

# A repo may map to more than one compose service (e.g. civilization ->
# civilization-server + civilization-web, both built from the same checkout).
# Falls back to [SERVICE_NAME] for repos that only ever mapped 1:1.
mapfile -t COMPOSE_SERVICES < <(jq -r --arg service "$SERVICE_NAME" '
    .[$service].compose_services // [$service] | .[]
' "$CONFIG_FILE")

# Rebuild and restart each compose service from this checkout.
for COMPOSE_SERVICE in "${COMPOSE_SERVICES[@]}"; do
    echo "Rebuilding container: $COMPOSE_SERVICE"
    docker compose -p "$PROJECT_NAME" -f "$COMPOSE_FILE" up --no-deps --build -d "$COMPOSE_SERVICE"
done

post_status success "Deployed $TAG"

send_notification \
    "[OK] Rebuild complete: $SERVICE_NAME" \
    "Service: $SERVICE_NAME
Compose services: ${COMPOSE_SERVICES[*]}
Repository: $REPO_FULL_NAME
Tag: $TAG
Status: Running
Time: $(date -u '+%Y-%m-%d %H:%M:%S UTC')"

echo "=== Rebuild complete for $SERVICE_NAME ==="

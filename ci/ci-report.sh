#!/usr/bin/env bash
# ci-report — run a command for the current commit and report the result to
# GitHub as a commit status. For repos without a self-hosted runner (public
# repos in particular): you start the build, nothing on GitHub can.
#
#   git push && ci-report just ci
#   ci-report --context local/e2e -- npm test
#
# The commit must already be on GitHub (push first), or GitHub rejects the
# status. Run from inside the repo. The command runs under ci-run, so it
# queues behind any other build on this machine.
#
# Token: GITHUB_STATUS_TOKEN from the environment, or from
# ~/.config/home-ci/env (a line GITHUB_STATUS_TOKEN=..., chmod 600).
set -euo pipefail

CONTEXT="local/ci"
while [ $# -gt 0 ]; do
    case "$1" in
        --context) CONTEXT="$2"; shift 2 ;;
        --) shift; break ;;
        -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
        *) break ;;
    esac
done
if [ $# -eq 0 ]; then
    echo "usage: ci-report [--context NAME] [--] <command> [args...]" >&2
    exit 2
fi

SELF_DIR="$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"

if [ -z "${GITHUB_STATUS_TOKEN:-}" ] && [ -f "$HOME/.config/home-ci/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.config/home-ci/env"
fi
if [ -z "${GITHUB_STATUS_TOKEN:-}" ]; then
    echo "ci-report: GITHUB_STATUS_TOKEN is not set (see github-setup-guide.md)" >&2
    exit 2
fi

REMOTE="$(git remote get-url origin)"
if [[ "$REMOTE" =~ github\.com[:/]([^/]+/[^/]+)$ ]]; then
    REPO="${BASH_REMATCH[1]%.git}"
else
    echo "ci-report: origin is not a GitHub remote: $REMOTE" >&2
    exit 2
fi
SHA="$(git rev-parse HEAD)"

if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "ci-report: working tree has uncommitted changes — the result would not describe $SHA" >&2
    exit 2
fi

post_status() {
    local state="$1" description="$2"
    curl -fsS -o /dev/null -X POST \
        -H "Authorization: Bearer $GITHUB_STATUS_TOKEN" \
        -H "Accept: application/vnd.github+json" \
        "https://api.github.com/repos/$REPO/statuses/$SHA" \
        -d "$(jq -n --arg s "$state" --arg d "$description" --arg c "$CONTEXT" \
              '{state: $s, description: $d, context: $c}')"
}

if ! post_status pending "Running on $(hostname)"; then
    echo "ci-report: GitHub rejected the status for ${SHA:0:7} — is the commit pushed, and does the token cover $REPO?" >&2
    exit 2
fi

start=$SECONDS
status=0
"$SELF_DIR/ci-run.sh" "$@" || status=$?
took="$((SECONDS - start))s"

if [ "$status" -eq 0 ]; then
    post_status success "Passed in $took on $(hostname)"
    echo "ci-report: $REPO@${SHA:0:7} $CONTEXT: success"
else
    post_status failure "Failed (exit $status) after $took on $(hostname)"
    echo "ci-report: $REPO@${SHA:0:7} $CONTEXT: FAILED (exit $status)" >&2
fi
exit "$status"

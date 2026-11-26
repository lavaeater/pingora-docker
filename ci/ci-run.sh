#!/usr/bin/env bash
# ci-run — run a command while holding the machine-wide build lock.
#
#   ci-run just ci
#
# Every heavy build on this machine goes through the same lock: CI jobs from
# every repo's Actions runner (via this script) and webhook deploys
# (webhooks/rebuild.sh takes the same file from inside the container). So no
# matter how many repos are pushed at once, builds queue up and run one at a
# time instead of fighting over the CPU.
#
# The lock is released when this script exits, however that happens — a
# cancelled job cannot leave it stuck.
set -euo pipefail

if [ $# -eq 0 ]; then
    echo "usage: ci-run <command> [args...]" >&2
    exit 2
fi

SELF_DIR="$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"
LOCK="${HOME_CI_LOCK:-$SELF_DIR/../repos/.locks/build.lock}"

mkdir -p "$(dirname "$LOCK")"
if [ ! -e "$LOCK" ]; then
    (umask 000 && : >>"$LOCK")
fi

# Read-only open is enough for flock, and works when the webhook container
# (root) created the file first.
exec 9<"$LOCK"
if ! flock -n 9; then
    echo "ci-run: another build is running on this machine, waiting for it..."
    waited=$SECONDS
    flock 9
    echo "ci-run: got the lock after $((SECONDS - waited))s"
fi

# The child does not inherit the lock fd: a daemon it leaves behind (a build
# cache server, a dev server) must not keep the lock held after we are done.
status=0
"$@" 9<&- || status=$?
exit "$status"

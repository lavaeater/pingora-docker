#!/usr/bin/env bash
# add-runner — register a self-hosted GitHub Actions runner for one repo and
# run it as a systemd user service on this machine.
#
#   ci/add-runner.sh lavaeater/oxidian            # prompts for the token
#   ci/add-runner.sh lavaeater/oxidian <token>    # registration token from
#       GitHub: repo → Settings → Actions → Runners → New self-hosted runner
#   ci/add-runner.sh --remove lavaeater/oxidian [<removal-token>]
#
# Run it from your normal shell: the PATH of that shell (cargo, dx, just,
# node, the kache shims) is what jobs will see. Re-run it after changing PATH.
#
# Personal GitHub accounts have no account-wide runners, so this is one runner
# (and one copy of the runner software, in ~/actions-runners/) per repo. Jobs
# from different repos still run one at a time — see ci/ci-run.sh.
set -euo pipefail

RUNNERS_DIR="${RUNNERS_DIR:-$HOME/actions-runners}"
LABEL="home-ci"
SELF_DIR="$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"

REMOVE=false
if [ "${1:-}" = "--remove" ]; then
    REMOVE=true
    shift
fi
REPO="${1:-}"
TOKEN="${2:-}"
if [[ ! "$REPO" =~ ^[^/]+/[^/]+$ ]]; then
    sed -n '2,15p' "$0"
    exit 2
fi

INSTANCE="${REPO//\//.}"            # lavaeater/oxidian -> lavaeater.oxidian
DIR="$RUNNERS_DIR/$INSTANCE"
UNIT="gh-runner@$INSTANCE.service"

if $REMOVE; then
    systemctl --user disable --now "$UNIT" 2>/dev/null || true
    if [ -x "$DIR/config.sh" ]; then
        if [ -z "$TOKEN" ]; then
            echo "Removal token (repo → Settings → Actions → Runners → the runner → Remove):"
            read -r TOKEN
        fi
        (cd "$DIR" && ./config.sh remove --token "$TOKEN")
    fi
    echo "Runner for $REPO unregistered. Its files are still in $DIR — delete them when you no longer need the workspace."
    exit 0
fi

# ── Download the runner (cached, shared between repos) ───────────────────────
case "$(uname -m)" in
    x86_64)  ARCH=x64 ;;
    aarch64) ARCH=arm64 ;;
    *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
mkdir -p "$RUNNERS_DIR/.cache" "$DIR"

if [ ! -x "$DIR/config.sh" ]; then
    VERSION="$(curl -fsS https://api.github.com/repos/actions/runner/releases/latest | jq -r .tag_name)"
    VERSION="${VERSION#v}"
    TARBALL="$RUNNERS_DIR/.cache/actions-runner-linux-$ARCH-$VERSION.tar.gz"
    if [ ! -f "$TARBALL" ]; then
        echo "Downloading actions runner $VERSION..."
        curl -fL --progress-bar -o "$TARBALL.part" \
            "https://github.com/actions/runner/releases/download/v$VERSION/actions-runner-linux-$ARCH-$VERSION.tar.gz"
        mv "$TARBALL.part" "$TARBALL"
    fi
    tar -xzf "$TARBALL" -C "$DIR"
fi

# ── Register with the repo ───────────────────────────────────────────────────
if [ ! -f "$DIR/.runner" ]; then
    if [ -z "$TOKEN" ]; then
        echo "Registration token for $REPO"
        echo "(https://github.com/$REPO/settings/actions/runners/new — the value after --token):"
        read -r TOKEN
    fi
    (cd "$DIR" && ./config.sh --unattended \
        --url "https://github.com/$REPO" \
        --token "$TOKEN" \
        --name "$(hostname)-${REPO#*/}" \
        --labels "$LABEL" \
        --work _work \
        --replace)
else
    echo "Runner already registered in $DIR — refreshing the service only."
fi

# ── systemd user service ─────────────────────────────────────────────────────
# Jobs get the PATH of the shell that ran this script, plus ci/ so workflows
# can call `ci-run.sh` by name.
printf 'PATH=%s:%s\n' "$SELF_DIR" "$PATH" >"$DIR/service.env"

mkdir -p "$HOME/.config/systemd/user"
install -m 644 "$SELF_DIR/gh-runner@.service" "$HOME/.config/systemd/user/gh-runner@.service"
systemctl --user daemon-reload
systemctl --user enable "$UNIT"
systemctl --user restart "$UNIT"

if [ "$(loginctl show-user "$USER" -p Linger --value 2>/dev/null)" != "yes" ]; then
    echo "NOTE: lingering is off, so the runner stops when you log out. Enable with:"
    echo "  sudo loginctl enable-linger $USER"
fi

echo
echo "Runner for $REPO is running:  systemctl --user status $UNIT"
echo "Logs:                         journalctl --user -u $UNIT -f"
echo "Workflows select it with:     runs-on: [self-hosted, $LABEL]"

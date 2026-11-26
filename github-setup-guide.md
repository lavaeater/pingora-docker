# Local CI and deploys, reported to GitHub

Builds and tests run on the home machine, with its toolchain and caches.
GitHub only stores the code and shows the results.

| What | Triggered by | Runs | Shows on GitHub as |
|------|--------------|------|--------------------|
| **CI**, private repos | push / PR | self-hosted Actions runner → `just ci` | a normal Actions check |
| **CI**, public repos | you, after pushing | `ci-report.sh just ci` | commit status `local/ci` |
| **Deploy** | tag push → webhook | `webhooks/rebuild.sh` (Docker build + restart) | commit status `deploy/home` |

Everything in the middle column goes through one machine-wide lock, so builds
from different repos queue instead of running at once (see
[Several builds at once](#several-builds-at-once)).

Paths below assume this repo is at `~/projects/rust/dioxus/pingora-docker`,
called `$PD` from here on.

---

## One-time setup

### 1. Create the status token

GitHub → Settings → Developer settings → Personal access tokens →
**Fine-grained tokens** → Generate new token.

- **Repository access:** "Only select repositories" — the repos you want
  statuses on. (Add new repos to it later; no new token needed.)
- **Permissions:** Repository → **Commit statuses: Read and write**. Nothing else.

The token can only set the red/green marks on commits. It cannot read or push code.

### 2. Give the token to the deploy webhook

Add it to `$PD/.env`:

```sh
GITHUB_STATUS_TOKEN=github_pat_...
```

Then rebuild the webhook container (its image gained `curl`):

```sh
cd $PD
docker compose up --no-deps --build -d webhook
```

> Restarting the webhook container checks out the **latest tag** in every repo
> under `repos/` (it does not rebuild the services). That is expected.

### 3. Give the token to `ci-report.sh`

Only needed if you will use `ci-report.sh` (the public-repo path).

```sh
mkdir -p ~/.config/home-ci
echo 'GITHUB_STATUS_TOKEN=github_pat_...' > ~/.config/home-ci/env
chmod 600 ~/.config/home-ci/env
ln -s $PD/ci/ci-report.sh ~/.local/bin/ci-report
ln -s $PD/ci/ci-run.sh    ~/.local/bin/ci-run
```

---

## Per repo: CI on a private repo

### 1. Add a `just ci` recipe

Copy `$PD/ci/templates/justfile` into the repo and make `ci` run everything
that should gate a merge. Check it locally:

```sh
just ci
```

This recipe is the single definition of "CI passes". The runner, `ci-report`
and you all run the same thing.

### 2. Register a runner for the repo

On GitHub: repo → Settings → Actions → Runners → **New self-hosted runner**.
Copy the token shown after `--token` in the "Configure" block (valid for one hour).

From your normal shell (jobs inherit this shell's `PATH`):

```sh
$PD/ci/add-runner.sh lavaeater/<repo> <token>
```

This downloads the runner into `~/actions-runners/lavaeater.<repo>/`,
registers it with the label `home-ci`, and starts it as the systemd user
service `gh-runner@lavaeater.<repo>`. The runner should now show as **Idle**
on the repo's Runners page.

Personal accounts cannot share one runner across repos, so this step is once
per repo. Each runner is idle until its repo has a job.

### 3. Add the workflow

Copy `$PD/ci/templates/ci.yml` to `.github/workflows/ci.yml` in the repo,
replacing any existing GitHub-hosted workflow. Commit and push. The run
appears under the repo's Actions tab and as a check on the commit or PR.

Things the template does that matter:

- `runs-on: [self-hosted, home-ci]` — never falls back to paid GitHub runners.
- `clean: false` on checkout — keeps `target/` between runs, so builds are incremental.
- `ci-run.sh just ci` — takes the machine-wide build lock.
- `concurrency` — a new push cancels the superseded run on the same branch.

### 4. Optional: require the check

Repo → Settings → Branches → branch protection for `main` → "Require status
checks to pass" → select **CI (home)**. Remember the check can only pass while
the machine is on.

### Who can run code on the machine

A job runs as your user, with access to your home directory. On a private
repo that means: **anyone with write access to the repo can run code on your
machine.** For repos only you push to, that is just you. Do not register a
runner on a repo with collaborators you would not give a shell to.

Private repos do not run workflows for fork pull requests unless you enable
it (Settings → Actions → General → "Fork pull request workflows"). Leave it off.

---

## Per repo: CI on a public repo

**Do not register a self-hosted runner on a public repo.** Anyone can open a
pull request that changes the workflow file, and the runner would execute it
as your user. Use one of these instead.

### Option A: GitHub-hosted runners

Standard GitHub-hosted runners are free for public repos. Keep
`runs-on: ubuntu-latest`. Slower, but nothing of yours is exposed.

### Option B: `ci-report` — you start the build

Nothing on GitHub can trigger this. You run it; it posts the result.

```sh
git push && ci-report just ci
```

The commit gets a `local/ci` status: pending while it runs, then
success/failure with the duration. It appears on the commit and on any PR
containing it.

- The repo must be in the token's repository list (one-time setup, step 1).
- The commit must be pushed first — GitHub rejects statuses for commits it does not have.
- It refuses to run with uncommitted changes, since the result would not describe the commit.
- `--context local/e2e` reports a second, separate check.

To make it a habit, add a git alias:

```sh
git config --global alias.pushci '!git push "$@" && ci-report just ci #'
```

To test someone else's pull request, read the diff first, check it out, and
run `ci-report just ci` yourself. That review step is the security boundary.

### If you put a runner on a public repo anyway

GitHub advises against it. If you do, all of these are needed:

1. Repo → Settings → Actions → General → "Approval for running fork pull
   request workflows from contributors" → **Require approval for all external
   contributors**. Then read the diff, including `.github/`, before approving.
2. Trigger the workflow on `push` only — remove `pull_request`.
3. Never use `pull_request_target` with a checkout of the PR's code.

The `if:` guard in the template is not protection; a fork can delete it.

---

## Per repo: deploys

Deploys already work through the webhook: a **tag push** rebuilds the
service's container. With `GITHUB_STATUS_TOKEN` set, the tagged commit also
gets a `deploy/home` status (pending → success/failure).

To add a repo:

1. Add the service to `docker-compose.yml` and the domain to `config.json`
   (see `readme.md`, "Adding a new service").
2. Add the repo to `webhooks/services.json`, then `docker restart webhook`.
3. On GitHub: repo → Settings → Webhooks → Add webhook. Payload URL
   `https://webhooks.kidvhs.com/hooks/rebuild-service`, content type
   `application/json`, secret = `WEBHOOK_SECRET` from `.env`, "Just the push event".
4. Add the repo to the status token's repository list.
5. Release with `git tag v1.2.3 && git push --tags`.

This is safe for public repos too. The webhook only acts on HMAC-signed
requests, only for repos listed in `services.json`, and only on tag pushes —
which need write access to the repo. A fork or a pull request cannot trigger it.

---

## Several builds at once

Each runner handles one job at a time, but with one runner per repo, pushing
to three repos would start three Rust builds at once. Two mechanisms prevent that:

- **Across repos, and between CI and deploys:** `ci-run.sh` and `rebuild.sh`
  take the same lock file (`repos/.locks/build.lock`). One build runs; the
  rest wait in arrival order and start as soon as it finishes. A waiting CI
  job shows as "in progress" on GitHub with
  `ci-run: another build is running on this machine, waiting for it...` in its
  log. A cancelled or crashed build releases the lock automatically.
- **Within one repo:** the workflow's `concurrency` group cancels a run when a
  newer push to the same branch arrives, so you only build the latest commit.

Waiting counts against the workflow's `timeout-minutes` (90 in the template).
Raise it if you routinely queue long builds.

The dev work you do by hand (`cargo build`, `dx serve`, bacon) does not take
the lock. Runners are started with low CPU and IO priority, so a CI build in
the background should not make the desktop sluggish. Deploy builds run in
Docker and are not deprioritised.

---

## Maintenance

| Task | Command |
|------|---------|
| Runner status | `systemctl --user status 'gh-runner@*'` |
| Runner logs | `journalctl --user -u gh-runner@lavaeater.<repo> -f` |
| Changed your `PATH` / installed a new tool | re-run `add-runner.sh lavaeater/<repo>` (no token needed) |
| Remove a runner | `$PD/ci/add-runner.sh --remove lavaeater/<repo>` |
| Deploy logs | `docker logs -f webhook` |
| Wipe a runner's build cache | `rm -rf ~/actions-runners/lavaeater.<repo>/_work` |

The runner software updates itself. Each runner keeps its own checkout and
`target/` under `_work/`, separate from your working copy.

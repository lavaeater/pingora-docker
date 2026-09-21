# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a home server infrastructure project using Cloudflare Pingora (Rust) as a reverse proxy, orchestrated with Docker Compose (single-node) and, increasingly, Docker Swarm (multi-node — see `SWARM_SETUP.md`). The Pingora proxy handles TLS termination and domain-based routing to various self-hosted services.

Phase 1 target: the Raspberry Pi runs everything in `docker-compose.yml`/`docker-stack.yml` except `jellyfin` (which needs the workstation's GPU for hardware transcoding). Longer term, as more nodes join the swarm, services move off the Pi until it runs only `pingora`.

## Build & Run Commands

```bash
# Build all containers (single-node, docker-compose.yml)
docker compose build

# Start all services
docker compose up -d

# Build and restart a single service
docker compose up --no-deps --build <service> -d

# Deploy/update the multi-node stack (docker-stack.yml, see SWARM_SETUP.md)
docker stack deploy -c docker-stack.yml pingora

# Build only the Rust proxy
cargo build --release

# Build the certificate provisioning tool
cargo build --release --bin provision
```

## Architecture

### Pingora Proxy (Rust)
- `src/main.rs` — Entry point: loads `config.json`, optionally provisions ACME certs, starts HTTP/HTTPS listeners
- `src/proxy.rs` — Core routing: `DomainRouter` implements Pingora's service trait, routes by Host header using `ProxyConfig`/`BackendConfig`
- `src/acme.rs` — Let's Encrypt certificate provisioning via DuckDNS DNS validation
- `src/provision.rs` — Standalone binary for certificate management

### Configuration
- `config.json` — Production routing config (domain → backend host:port mappings, TLS settings)
- `config.example.json` — Template for new deployments
- `.env` — Secrets: `WEBHOOK_SECRET`, default credentials, SMTP settings

### Services (docker-compose.yml / docker-stack.yml)
All on custom `proxy-network` (bridge in Compose, overlay in Swarm):

| Service | Port | Purpose |
|---------|------|---------|
| pingora | 8080/8443 | Reverse proxy |
| webhook | 9000 | GitHub webhook → auto-rebuild |
| jellyfin | 8096 | Media streaming (workstation only — needs GPU passthrough) |
| medusa | 8081 | TV show management |
| rusty-budgets | 8666 | Budget tracking (Rust) |
| oxidian | 5173 | Obsidian vault browser (Rust) |
| oxidize-books | 8888 | E-book management (Rust + PostgreSQL) |
| postgres | 5432 | DB for oxidize-books, rusty-budgets |
| deluge | 8112/6881 | Torrent client |

### Webhook Auto-Rebuild
- `webhooks/rebuild.sh` (Compose): triggered by GitHub push events (HMAC-validated). Pulls latest code into `./repos/<service>/` and runs `docker compose up --no-deps --build <service> -d`.
- `webhooks/swarm-rebuild.sh` (Swarm): triggered on tag pushes. Builds the image, pushes it to the private registry, and runs `docker service update`/`docker stack deploy`.
- Both read service repo/build info from `webhooks/services.json`.

## Domain Routing
Domains resolve to container names on the proxy network. To add a new service:
1. Add container to `docker-compose.yml` with a service name
2. Add domain → `<container-name>:<port>` mapping in `config.json`
3. Rebuild and restart pingora

# Pingora Domain-Based Reverse Proxy

A high-performance reverse proxy built on [Cloudflare's Pingora](https://github.com/cloudflare/pingora) that routes traffic to different Docker containers based on the incoming domain name.

## Features

- **Domain-based routing**: Route traffic to different backend services based on the Host header
- **Wildcard domains**: Support for `*.example.com` style wildcard matching
- **Session isolation**: Each domain maintains separate sessions/cookies (handled by browsers automatically)
- **Docker-native**: Works seamlessly with Docker container names for service discovery
- **Hot-reloadable config**: Update `config.json` and restart to apply changes
- **TLS support**: Optional TLS for backend connections
- **Default backend**: Fallback for unmatched domains

## Quick Start

### 1. Configure your domains

Edit `config.json` to map your domains to backend services:

```json
{
    "listen_addr": "0.0.0.0:8080",
    "domains": {
        "app1.yourdomain.com": {
            "host": "webapp1",
            "port": 3000,
            "tls": false
        },
        "app2.yourdomain.com": {
            "host": "webapp2",
            "port": 8080,
            "tls": false
        }
    },
    "default_backend": {
        "host": "default-service",
        "port": 80,
        "tls": false
    }
}
```

### 2. Run with Docker Compose

```bash
docker compose build
docker compose up -d
```

### 3. Configure your router

Point your domain (e.g., `cleverdomain.asuscomm.com`) to your router's external IP, and forward port 8080 to the machine running this proxy.

## Configuration Reference

### Main Config (`config.json`)

| Field | Type | Description |
|-------|------|-------------|
| `listen_addr` | string | Address and port to listen on (e.g., `"0.0.0.0:8080"`) |
| `domains` | object | Map of domain names to backend configurations |
| `default_backend` | object | Optional fallback backend for unmatched domains |

### Backend Config

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `host` | string | required | Backend hostname (Docker container name or IP) |
| `port` | number | required | Backend port |
| `tls` | boolean | `false` | Use TLS when connecting to backend |
| `sni` | string | `host` | SNI hostname for TLS connections |

### Wildcard Domains

You can use `*` as a prefix to match subdomains:

```json
{
    "domains": {
        "*.internal.local": {
            "host": "internal-gateway",
            "port": 8080,
            "tls": false
        }
    }
}
```

This matches `foo.internal.local`, `bar.internal.local`, etc.

## Session & Login Isolation

Sessions and logins are **automatically isolated per domain** because:

1. Browsers scope cookies by domain
2. Each domain routes to a different backend service
3. The proxy preserves the original `Host` header

So a user logged into `app1.yourdomain.com` will have a completely separate session from `app2.yourdomain.com`.

## Example Setup

See `docker-compose.example.yml` for a complete example with multiple backend services.

```bash
# Copy example files
cp config.example.json config.json
cp docker-compose.example.yml docker-compose.yml

# Create example content directories
mkdir -p examples/{webapp1,webapp2,api,default}
echo '<h1>App 1</h1>' > examples/webapp1/index.html
echo '<h1>App 2</h1>' > examples/webapp2/index.html
echo '{"status": "ok"}' > examples/api/index.html
echo '<h1>Default</h1>' > examples/default/index.html

# Start everything
docker compose up -d
```

## Network Architecture

```
Internet
    │
    ▼ (port 8080)
┌─────────────────────────────────────────────────────────────┐
│  Router (cleverdomain.asuscomm.com:8080)                    │
└─────────────────────────────────────────────────────────────┘
    │
    ▼ (port forward)
┌─────────────────────────────────────────────────────────────┐
│  Pingora Reverse Proxy (Docker: pingora-proxy)              │
│  - Inspects Host header                                     │
│  - Routes to appropriate backend                            │
└─────────────────────────────────────────────────────────────┘
    │
    ├──► app1.cleverdomain.com ──► webapp1:3000
    ├──► app2.cleverdomain.com ──► webapp2:8080  
    ├──► api.cleverdomain.com  ──► api-service:5000
    └──► (default)             ──► default-service:80
```

## Development

### Build locally

```bash
cargo build --release
```

### Run locally

```bash
RUST_LOG=info ./target/release/pingora
```

### Logging

Set `RUST_LOG` environment variable:
- `RUST_LOG=debug` - Verbose logging
- `RUST_LOG=info` - Normal operation logging
- `RUST_LOG=warn` - Warnings and errors only

## Adding to Existing Docker Compose

To add this proxy to an existing `docker-compose.yml`:

1. Add the proxy service and ensure it's on the same network as your backend services
2. Use container names as the `host` in your config

```yaml
services:
  reverse-proxy:
    image: your-registry/pingora-proxy:latest
    volumes:
      - ./config.json:/usr/src/pingora/config.json:ro
    ports:
      - "8080:8080"
    networks:
      - your-existing-network

  your-existing-service:
    # ... your existing config
    networks:
      - your-existing-network

networks:
  your-existing-network:
    external: true  # if already exists
```

## License

Apache License, Version 2.0

## Credits

Built on [Cloudflare Pingora](https://github.com/cloudflare/pingora) - a battle-tested framework serving 40M+ requests/second.
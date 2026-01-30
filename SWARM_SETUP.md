# Docker Swarm Setup Guide

This guide explains how to set up Docker Swarm for the pingora-docker project across multiple nodes.

## Node Distribution

| Node | Role Label | Services |
|------|------------|----------|
| Raspberry Pi | `rpi` | pingora, webhook, registry |
| Old Laptop | `laptop` | oxidize-books, rusty-budgets, postgres |
| Workstation | `workstation` | jellyfin, sickgear, deluge |

## Prerequisites

- Docker installed on all nodes
- Network connectivity between all nodes (same network or VPN)
- Open ports: 2377/tcp (cluster management), 7946/tcp+udp (node communication), 4789/udp (overlay network)

## Step 1: Initialize the Swarm (on Raspberry Pi)

The Raspberry Pi will be the **manager node**:

```bash
docker swarm init --advertise-addr <RPI_IP_ADDRESS>
```

This outputs a join token. Save it for the worker nodes.

## Step 2: Join Worker Nodes

On the **laptop** and **workstation**, run the join command from step 1:

```bash
docker swarm join --token <TOKEN> <RPI_IP_ADDRESS>:2377
```

## Step 3: Label the Nodes

On the manager (Raspberry Pi), label each node:

```bash
# Get node IDs
docker node ls

# Label the Raspberry Pi (manager)
docker node update --label-add role=rpi <RPI_NODE_ID>

# Label the laptop
docker node update --label-add role=laptop <LAPTOP_NODE_ID>

# Label the workstation
docker node update --label-add role=workstation <WORKSTATION_NODE_ID>
```

## Step 4: Private Registry

The stack includes a **private registry** running on the Raspberry Pi (port 5000). This stores your custom-built images.

### Configure Docker to Trust the Registry

On **each node** (laptop, workstation), add the registry as an insecure registry:

```bash
# Edit /etc/docker/daemon.json
sudo nano /etc/docker/daemon.json
```

Add:
```json
{
  "insecure-registries": ["<RPI_IP_ADDRESS>:5000"]
}
```

Then restart Docker:
```bash
sudo systemctl restart docker
```

### Alternative: Secure with TLS

For production, generate TLS certificates and configure the registry with HTTPS. See [Docker Registry TLS docs](https://docs.docker.com/registry/deploying/#run-an-externally-accessible-registry).

## Step 5: Build and Push Custom Images

From the project directory on a machine with the source code:

```bash
# Set your registry (use the Pi's IP so other nodes can pull)
export REGISTRY=<RPI_IP_ADDRESS>:5000

# Build images
docker build -t $REGISTRY/pingora:latest .
docker build -t $REGISTRY/webhook:latest ./webhooks
docker build -t $REGISTRY/rusty-budgets:latest ./repos/rusty-budgets
docker build -t $REGISTRY/oxidize-books:latest ./repos/oxidize-books

# Push to registry
docker push $REGISTRY/pingora:latest
docker push $REGISTRY/webhook:latest
docker push $REGISTRY/rusty-budgets:latest
docker push $REGISTRY/oxidize-books:latest
```

## Step 6: Prepare Volumes on Each Node

### On Workstation (jellyfin, sickgear, deluge)
Ensure these paths exist:
- `/media/brontosaurus/media`
- `/home/tommie/Downloads/deluge`

### On Laptop (oxidize-books, rusty-budgets, postgres)
The stack uses named volumes, which Docker manages automatically.

### On Raspberry Pi (pingora, webhook)
Copy the config files:
```bash
# Ensure these exist on the Pi
./config.json
./certs/
./webhooks/hooks.json
./webhooks/services.json
./webhooks/rebuild.sh
./webhooks/init-services.sh
```

## Step 7: Deploy the Stack

From the manager node (Raspberry Pi):

```bash
# Set environment variables
export REGISTRY=localhost:5000
export WEBHOOK_SECRET=your_secret_here

# Deploy
docker stack deploy -c docker-stack.yml pingora
```

## Managing the Stack

```bash
# View services
docker service ls

# View service logs
docker service logs pingora_pingora

# Scale a service
docker service scale pingora_oxidize-books=2

# Update the stack
docker stack deploy -c docker-stack.yml pingora

# Remove the stack
docker stack rm pingora
```

## Handling Workstation Reboots (Linux/Windows dual-boot)

When the workstation reboots or switches OS:
- Swarm will mark the node as `Down`
- Services (jellyfin, sickgear, deluge) become unavailable
- When the node rejoins, services automatically restart

To gracefully handle planned downtime:
```bash
# On manager, before reboot
docker node update --availability drain <WORKSTATION_NODE_ID>

# After reboot and rejoining
docker node update --availability active <WORKSTATION_NODE_ID>
```

## Troubleshooting

### Node not joining
- Check firewall rules for ports 2377, 7946, 4789
- Verify network connectivity: `ping <MANAGER_IP>`

### Service not starting
```bash
docker service ps pingora_jellyfin --no-trunc
```

### Overlay network issues
```bash
docker network inspect pingora_proxy-network
```

## Network Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Docker Swarm Overlay Network                  │
│                       (proxy-network)                            │
├─────────────────┬─────────────────────┬─────────────────────────┤
│   Raspberry Pi  │     Old Laptop      │      Workstation        │
│   (Manager)     │     (Worker)        │      (Worker)           │
├─────────────────┼─────────────────────┼─────────────────────────┤
│ • pingora:8443  │ • oxidize-books:8888│ • jellyfin:8096         │
│ • webhook:9000  │ • rusty-budgets:8666│ • sickgear:8081         │
│ • registry:5000 │ • postgres:5432     │ • deluge:8112           │
└─────────────────┴─────────────────────┴─────────────────────────┘
```

All services can communicate via service names (e.g., `postgres`, `pingora`) through the overlay network.

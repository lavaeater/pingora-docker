# Docker Swarm Setup Guide

This guide explains how to set up Docker Swarm for the pingora-docker project across multiple nodes.

## Node Distribution

**Phase 1 (current):** only the Raspberry Pi and the workstation exist. Everything runs on
the Pi except `jellyfin`, which stays on the workstation because it needs the workstation's
GPU for hardware transcoding. The `laptop` role below is not in use yet — it's reserved for
when a dedicated machine is added, at which point services can be redistributed off the Pi
and, eventually, the Pi can go back to running `pingora` alone.

| Node | Role Label | Services |
|------|------------|----------|
| Raspberry Pi | `rpi` | pingora, webhook, registry, medusa, rusty-budgets, oxidian, oxidize-books, postgres, deluge |
| Workstation | `workstation` | jellyfin |
| Old Laptop (future) | `laptop` | not yet in use — see Phase 1 note above |

## SSH Setup for Remote Management

Configure SSH on all machines so you can manage everything from one keyboard.

### On Each Remote Machine (Raspberry Pi, Laptop, Workstation)

Install and enable SSH server:
```bash
# Arch Linux
sudo pacman -S openssh
sudo systemctl enable sshd
sudo systemctl start sshd
```

### On Your Main Machine (where you'll work from)

#### 1. Generate SSH Key (if you don't have one)
```bash
ssh-keygen -t ed25519 -C "your_email@example.com"
```

#### 2. Copy Your Key to Each Machine
```bash
ssh-copy-id tommie@rpi
ssh-copy-id tommie@laptop
ssh-copy-id tommie@workstation
```

Replace hostnames with IP addresses if DNS isn't configured.

#### 3. Configure SSH Aliases

Edit `~/.ssh/config`:
```
Host rpi
    HostName 192.168.1.10
    User tommie

Host laptop
    HostName 192.168.1.11
    User tommie

Host workstation
    HostName 192.168.1.12
    User tommie
```

Now you can simply run:
```bash
ssh rpi
ssh laptop
ssh workstation
```

#### 4. Optional: Disable Password Authentication (More Secure)

On each remote machine, edit `/etc/ssh/sshd_config`:
```
PasswordAuthentication no
PubkeyAuthentication yes
```

Then restart SSH:
```bash
sudo systemctl restart sshd
```

### Quick Commands from Your Main Machine

```bash
# Run a command on all nodes
for host in rpi laptop workstation; do
    echo "=== $host ===" && ssh $host "docker node ls" 
done

# Open multiple terminals (if using tmux)
tmux new-session -s swarm \; \
    send-keys "ssh rpi" C-m \; \
    split-window -h \; \
    send-keys "ssh laptop" C-m \; \
    split-window -v \; \
    send-keys "ssh workstation" C-m
```

## Prerequisites

- Docker installed on all nodes
- SSH access to all nodes (see above)
- Network connectivity between all nodes (same network or VPN)
- Open ports: 2377/tcp (cluster management), 7946/tcp+udp (node communication), 4789/udp (overlay network), 22/tcp (SSH)

## Step 1: Initialize the Swarm (on Raspberry Pi)

The Raspberry Pi will be the **manager node**:

```bash
docker swarm init --advertise-addr <RPI_IP_ADDRESS>
```

This outputs a join token. Save it for the worker nodes.

## Step 2: Join Worker Nodes

For Phase 1 you only need the **workstation** (for `jellyfin`); skip the laptop until you
actually have one to add. On each worker node, run the join command from step 1:

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

### Automated (via Webhooks)

The webhook service automatically builds and pushes images when you push a tag to GitHub. It uses `swarm-rebuild.sh` which:
1. Clones/pulls the repository
2. Builds the Docker image
3. Pushes to the private registry
4. Updates the Swarm service

**Setup:** Use `hooks-swarm.json.example` as your `hooks.json` template:
```bash
cp webhooks/hooks-swarm.json.example webhooks/hooks.json
# Edit hooks.json and replace YOUR_WEBHOOK_SECRET_HERE with your secret
```

### Manual (Initial Bootstrap)

For the initial deployment, build and push images manually:

```bash
# Set your registry (use the Pi's IP so other nodes can pull)
export REGISTRY=<RPI_IP_ADDRESS>:5000

# Build images
docker build -t $REGISTRY/pingora:latest .
docker build -t $REGISTRY/webhook:latest ./webhooks
docker build -t $REGISTRY/rusty-budgets:latest ./repos/rusty-budgets
docker build -t $REGISTRY/oxidize-books:latest ./repos/oxidize-books
docker build -t $REGISTRY/oxidian:latest ./repos/oxidian

# Push to registry
docker push $REGISTRY/pingora:latest
docker push $REGISTRY/webhook:latest
docker push $REGISTRY/rusty-budgets:latest
docker push $REGISTRY/oxidize-books:latest
docker push $REGISTRY/oxidian:latest
```

## Step 6: Prepare Volumes on Each Node

### On Raspberry Pi (Phase 1: everything except jellyfin)
The stack uses named volumes for `pingora`, `webhook`, `registry`, `medusa`, `rusty-budgets`,
`oxidian`, `oxidize-books`, and `postgres` — Docker manages those automatically. Also make
sure these host paths exist, since `medusa` and `deluge` bind directly to them:
- `/media/brontosaurus/media`
- `/home/tommie/Downloads/deluge`

Copy the config files:
```bash
# Ensure these exist on the Pi
./config.json
./certs/
./webhooks/hooks.json
./webhooks/services.json
./webhooks/rebuild.sh
./webhooks/swarm-rebuild.sh
./webhooks/init-services.sh
./docker-stack.yml
```

### On Workstation (jellyfin only, for now)
Ensure this path exists:
- `/media/brontosaurus/media`

### On Laptop (future)
Not in use yet — see the Phase 1 note under Node Distribution. Once added, `medusa`,
`rusty-budgets`, `oxidian`, `oxidize-books`, and `postgres` can be relabeled off the Pi
onto it; the stack already uses named volumes for these so no host paths need preparing.

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
- `jellyfin` becomes unavailable (everything else keeps running on the Pi)
- When the node rejoins, `jellyfin` automatically restarts

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

Phase 1 — only the Pi and workstation exist:

```
┌─────────────────────────────────────┬─────────────────────────┐
│              Raspberry Pi            │      Workstation        │
│              (Manager)                │      (Worker)           │
├───────────────────────────────────────┼─────────────────────────┤
│ • pingora:8443    • medusa:8081        │ • jellyfin:8096         │
│ • webhook:9000    • rusty-budgets:8666 │                         │
│ • registry:5000   • oxidian:5173       │                         │
│ • postgres:5432   • oxidize-books:8888 │                         │
│                    • deluge:8112       │                         │
└───────────────────────────────────────┴─────────────────────────┘
```

Once a laptop (or other dedicated) node joins, `medusa`, `rusty-budgets`, `oxidian`,
`oxidize-books`, `postgres`, and `deluge` can be relabeled onto it, freeing up the Pi —
eventually down to just `pingora`, per the Node Distribution note above.

All services can communicate via service names (e.g., `postgres`, `pingora`) through the overlay network.

## File Synchronization with Syncthing

To sync downloaded books from the workstation to the laptop (where oxidize-books processes them), use Syncthing.

### Install on Both Machines (Arch Linux)

```bash
sudo pacman -S syncthing
```

### Enable and Start the Service

Run as your user (not root):
```bash
systemctl --user enable syncthing
systemctl --user start syncthing
```

### Make It Survive Reboots

```bash
loginctl enable-linger $USER
```

This keeps user services running even when you're not logged in.

### Access the Web UI

Open `http://localhost:8384` in your browser on each machine.

### Configure the Sync

1. **On workstation:** Actions → Show ID, copy the Device ID
2. **On laptop:** Add Remote Device → paste workstation's Device ID
3. **On workstation:** Accept the laptop's connection request (or add laptop's ID manually)
4. **On workstation:** Add Folder → select `/home/tommie/Downloads/deluge/finished/books`
   - Set Folder Type to **"Send Only"**
   - In Sharing tab, check the laptop device
5. **On laptop:** Accept the folder share
   - Set local path to `/home/tommie/books_incoming` (or wherever oxidize-books expects input)
   - Set Folder Type to **"Receive Only"**

### Verify It's Running

```bash
systemctl --user status syncthing
```

### How It Works

```
┌─────────────────────────────────────────────────────────────────┐
│                         Syncthing                                │
├────────────────────────────┬────────────────────────────────────┤
│        Workstation         │            Laptop                  │
│    (Send Only)             │        (Receive Only)              │
├────────────────────────────┼────────────────────────────────────┤
│ ~/Downloads/.../books/     │ ~/books_incoming/                  │
│         ↓                  │         ↓                          │
│   [new book.epub]    ───────────→  [new book.epub]              │
│                            │         ↓                          │
│                            │   oxidize-books processes it       │
└────────────────────────────┴────────────────────────────────────┘
```

Files dropped in the books folder on workstation automatically sync to the laptop over LAN.

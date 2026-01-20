#!/bin/bash

# Setup SSH with correct permissions
mkdir -p /root/.ssh
cp /ssh-keys/id_ed25519 /root/.ssh/id_ed25519
cp /ssh-keys/known_hosts /root/.ssh/known_hosts
chmod 600 /root/.ssh/id_ed25519
chmod 644 /root/.ssh/known_hosts

# Execute the webhook binary with all passed arguments
exec /usr/local/bin/webhook "$@"

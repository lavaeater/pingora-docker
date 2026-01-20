FROM almir/webhook

# Install dependencies needed for rebuild script
RUN apk add --no-cache \
    git \
    jq \
    docker-cli \
    docker-cli-compose \
    bash

# Make rebuild script executable
RUN chmod +x /scripts/rebuild.sh 2>/dev/null || true

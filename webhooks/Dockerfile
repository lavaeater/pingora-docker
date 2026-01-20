FROM almir/webhook

USER root

# Install dependencies needed for rebuild script
RUN apk add --no-cache \
    git \
    jq \
    docker-cli \
    docker-cli-compose \
    bash \
    openssh-client

# Make rebuild script executable
RUN chmod +x /scripts/rebuild.sh 2>/dev/null || true

# Copy entrypoint script
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]

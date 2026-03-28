#!/bin/bash

# Setup SSH with correct permissions
mkdir -p /root/.ssh
cp /ssh-keys/id_ed25519 /root/.ssh/id_ed25519
cp /ssh-keys/known_hosts /root/.ssh/known_hosts
chmod 600 /root/.ssh/id_ed25519
chmod 644 /root/.ssh/known_hosts

# Configure msmtp for email notifications if SMTP settings are provided
if [ -n "$SMTP_HOST" ]; then
    cat > /etc/msmtprc <<EOF
defaults
auth           on
tls            on
tls_starttls   ${SMTP_STARTTLS:-on}
logfile        /var/log/msmtp.log

account        default
host           $SMTP_HOST
port           ${SMTP_PORT:-587}
from           $SMTP_FROM
user           $SMTP_USER
password       $SMTP_PASSWORD
EOF
    chmod 600 /etc/msmtprc
    ln -sf /usr/bin/msmtp /usr/sbin/sendmail
    echo "msmtp configured for $SMTP_HOST"
fi

# Run init script on startup if INIT_SERVICES is set
if [ "$INIT_SERVICES" = "true" ]; then
    echo "=== Running service initialization ==="
    /scripts/init-services.sh ${BUILD_ON_INIT:+--build}
fi

# Execute the webhook binary with all passed arguments
exec /usr/local/bin/webhook "$@"

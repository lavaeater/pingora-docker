#!/bin/bash
# Creates one database per comma-separated name in $POSTGRES_MULTIPLE_DATABASES.
# Runs once, only against a fresh (empty) postgres data directory/volume —
# official postgres image convention for /docker-entrypoint-initdb.d/*.sh.
set -e

if [ -z "$POSTGRES_MULTIPLE_DATABASES" ]; then
    exit 0
fi

for db in $(echo "$POSTGRES_MULTIPLE_DATABASES" | tr ',' ' '); do
    echo "Creating database '$db' (if it doesn't already exist)"
    psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" <<-EOSQL
        SELECT 'CREATE DATABASE $db' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = '$db')\gexec
EOSQL
done

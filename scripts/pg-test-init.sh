#!/bin/sh
# Runs once when the test Postgres is first created (compose.yaml).
# Adds a passwordless role the server trusts, for the "empty password, server uses trust" test.
set -e
psql -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" -c 'CREATE ROLE tablex_trust LOGIN'
sed -i '1i host all tablex_trust all trust' "$PGDATA/pg_hba.conf"

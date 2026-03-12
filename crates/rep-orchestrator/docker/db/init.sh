#!/bin/bash
set -e

# Create users and databases
psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "postgres" <<-EOSQL
    CREATE USER $SERVICE_USER WITH PASSWORD '$SERVICE_PASSWORD';
    CREATE DATABASE $DB_NAME;
EOSQL

# Set permissions on the new database
psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$DB_NAME" <<-EOSQL
    GRANT CONNECT ON DATABASE $DB_NAME TO $SERVICE_USER;
    GRANT USAGE, CREATE ON SCHEMA public TO $SERVICE_USER;
EOSQL

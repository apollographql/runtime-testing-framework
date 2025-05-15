#!/usr/bin/env sh
# Run a local server using the given credentials

# Parse the inputs we received from RTF
SERVER_CMD="$(echo "$1" | jq -r '.server_command')"
AUTH_HEADER="$(echo "$1" | jq -r '.auth_header')"
export AUTH_HEADER

# Run the server and grab its pid
$SERVER_CMD &
PID="$!"

# Output the details we need to satisfy the base test plan
jq -n -c --arg pid $PID \
  '{
    "server_url": "127.0.0.1:8000",
    "server_pid": $pid
  }'

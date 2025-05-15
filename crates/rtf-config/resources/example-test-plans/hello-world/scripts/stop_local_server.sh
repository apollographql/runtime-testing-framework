#!/usr/bin/env sh
# Stop a local server

# Parse the inputs we received from RTF
SERVER_PID="$(echo "$1" | jq -r '.server_pid')"
echo "server pid is $SERVER_PID"

# Kill the server process
kill "$SERVER_PID"

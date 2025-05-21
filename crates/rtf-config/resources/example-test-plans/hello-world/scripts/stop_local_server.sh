#!/usr/bin/env sh
# Stop a local server

# Parse the inputs we received from RTF
echo "server pid is $SERVER_PID"

# Kill the server process
kill "$SERVER_PID"

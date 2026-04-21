#!/usr/bin/env sh
set -e

# Wait for service to be ready
sleep 2

docker ps

# Curl the echo server
response=$(curl -v http://localhost:8083/)

echo "Response from echo-server:"
echo "$response"
echo "$response" >> "$OUTDIR/scenario.txt"

# Verify expected content
if echo "$response" | grep -q "env: ${EXPECTED_MESSAGE}"; then
    echo "SUCCESS: Environment variable echoed correctly"
else
    echo "FAIL: Expected message '${EXPECTED_MESSAGE}' not found"
    exit 1
fi

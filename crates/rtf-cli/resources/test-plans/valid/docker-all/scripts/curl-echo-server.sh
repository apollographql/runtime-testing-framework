#!/usr/bin/env sh
set -e

echo "installing curl"
apk add curl

echo "waiting for service to be ready"
sleep 2

echo "making request to server"
response=$(curl -v echo-server:8080/)

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

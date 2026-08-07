#!/usr/bin/env bash
set -euo pipefail

echo "Deleting kind clusters..."
kind delete cluster --name rtf-mgmt 2>/dev/null || true
kind delete cluster --name rtf-workload 2>/dev/null || true
echo "Done."

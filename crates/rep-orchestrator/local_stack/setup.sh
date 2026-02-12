#!/usr/bin/env bash
set -euo pipefail

echo "Checking local stack dependencies..."
dependencies="docker gcloud k9s kind rtf tilt"

missing=()
for dependency in $dependencies; do
  if ! which "${dependency}" > /dev/null 2>&1; then
    missing+=("${dependency}")
  fi
done

if (( ${#missing[@]} != 0 )); then
  echo "You need the following components to use this command:"
  echo "    $dependencies"
  echo "You are missing: ${missing[*]}"
  echo ""
  echo "Other than rtf, these components can be installed from homebrew."
  echo "  NOTE: k9s needs to be installed as 'derailed/k9s/k9s'"
  exit 1
fi

echo "Creating kind clusters..."
kind create cluster --name "rtf-mgmt"
kind create cluster --name "rtf-workload"
echo ""
echo "  Management cluster context: kind-rtf-mgmt"
echo "  Workload cluster context:   kind-rtf-workload"

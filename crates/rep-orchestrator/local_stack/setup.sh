#!/usr/bin/env bash
set -euo pipefail

cluster_exists() {
  kind get clusters 2>/dev/null | grep -qx "$1"
}

create_cluster() {
  if cluster_exists "$1"; then
    echo "Cluster '$1' already exists, skipping creation"
  else
    kind create cluster --name "$1"
  fi
}

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
create_cluster "rtf-mgmt"
create_cluster "rtf-workload"
echo ""
echo "  Management cluster context: kind-rtf-mgmt"
echo "  Workload cluster context:   kind-rtf-workload"

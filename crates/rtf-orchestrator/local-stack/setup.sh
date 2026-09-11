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
dependencies="docker kind kubectl helm tilt"

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
  echo "These components can be installed from homebrew."
  exit 1
fi

echo "Configuring Helm repos..."
helm repo add bitnami https://charts.bitnami.com/bitnami
helm repo update bitnami
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
helm dependency update "$SCRIPT_DIR/helm/postgres"

echo "Creating kind clusters..."
create_cluster "rtf-mgmt"
create_cluster "rtf-workload"

echo "Creating workload Cluster Roles"
kubectl apply --context kind-rtf-workload -f k8s/workload-rbac-clusterroles.yaml

REPO_ROOT="$(git rev-parse --show-toplevel)"
echo "Building toolbox image..."
docker build -t rtf-toolbox:edge -f "$REPO_ROOT/toolbox/Dockerfile" "$REPO_ROOT"
echo ""
echo "  Management cluster context: kind-rtf-mgmt"
echo "  Workload cluster context:   kind-rtf-workload"

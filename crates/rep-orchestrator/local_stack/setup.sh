#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

MGMT_CLUSTER="rtf-mgmt"
WORKLOAD_CLUSTER="rtf-workload"
MGMT_CONTEXT="kind-${MGMT_CLUSTER}"
WORKLOAD_CONTEXT="kind-${WORKLOAD_CLUSTER}"


cluster_exists() {
  kind get clusters 2>/dev/null | grep -qx "$1"
}

create_cluster() {
  if cluster_exists "$1"; then
    echo "Cluster '$1' already exists, skipping creation"
  else
    echo "Creating cluster '$1'..."
    kind create cluster --config "$SCRIPT_DIR/kind/$2"
  fi
}

create_namespace() {
  kubectl --context "$MGMT_CONTEXT" create namespace "$1" --dry-run=client -o yaml \
    | kubectl --context "$MGMT_CONTEXT" apply -f -
}


echo "Creating kind clusters..."
create_cluster "$MGMT_CLUSTER" management.yaml
create_cluster "$WORKLOAD_CLUSTER" workload.yaml


echo "Creating namespaces in management cluster..."
create_namespace argo
create_namespace cluster-api


echo "Local stack setup complete."
echo ""
echo "  Management cluster context: $MGMT_CONTEXT"
echo "  Workload cluster context:   $WORKLOAD_CONTEXT"

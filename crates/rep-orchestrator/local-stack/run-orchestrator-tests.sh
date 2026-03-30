#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

kubectl exec -i -n orchestrator --context kind-rtf-mgmt statefulset/postgres -- \
    env PGPASSWORD=password psql -U service_user rtf_rep \
    < "$SCRIPT_DIR/db/clear.sql"

RTF_DB_HOST=localhost \
RTF_DB_PORT=5432 \
RTF_DB_NAME=rtf_rep \
RTF_DB_USER=service_user \
RTF_DB_PASS=password \
RTF_APOLLO_KEY=dummy \
RTF_GITHUB_TOKEN=dummy \
RTF_KUBECONFIG_PATH=/root/.kube/config \
RTF_MGMT_CONTEXT=kind-rtf-mgmt \
RTF_WORKLOAD_CONTEXT=kind-rtf-workload \
    cargo test --manifest-path "$SCRIPT_DIR/../Cargo.toml" --features k8s_tests

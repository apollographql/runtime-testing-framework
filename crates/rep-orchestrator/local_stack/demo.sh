#!/usr/bin/env bash
# Demo of the REP based execution flow for RTF
#
# The scenario job being used here is still hard coded for the rtf-mcp
# smoke test test plan due to the need to customise the script that we run
# based on the RTF scenario config.
# Having this proved out for a single test plan is the goal so we can get
# started with working on the logic needed in the orchestrator for templating
# things out the job per-run.

set -e

if [ "$#" -ne 1 ]; then
  echo "Usage: $0 <path to rtf-mcp smpoke test test plan>"
  exit 1
fi

TEST_PLAN="$1"
DEPENDENCIES="kubectl rtf yq"
MISSING=()

echo "Checking dependencies..."

for dependency in $DEPENDENCIES; do
  if ! which "${dependency}" > /dev/null 2>&1; then
    MISSING+=("${dependency}")
  fi
done

if (( ${#MISSING[@]} != 0 )); then
  echo "You are missing the following required binaries on your path:"
  echo "  ${MISSING[*]}"
  exit 1
fi

echo "Inlining test plan from $TEST_PLAN..."
rtf inline all "$TEST_PLAN" --outdir rtf_data

echo "Extracting environment and scenario sections..."
yq '.environment' "rtf_data/inlined-test-plan.yaml" > rtf_data/environment.yaml
yq '.scenario' "rtf_data/inlined-test-plan.yaml" > rtf_data/scenario.yaml

echo "Creating RTF environment data configmap..."
kubectl -n cluster-api --context kind-rtf-mgmt \
  create configmap rtf-environment-config \
  --from-file=rtf_data/environment.yaml

sleep 2

echo "Running argo workflow to provision test namespace..."
kubectl -n cluster-api --context kind-rtf-mgmt \
  create -f demo/environment-workflow.yaml

echo "Waiting for environment provisioning to complete..."
# FIXME: really we want to wait on Completed OR Failed here
kubectl -n cluster-api --context kind-rtf-mgmt \
  wait --for=condition=Completed \
  workflow/provision-rtf-environment \
  --timeout=600s

echo "Creating RTF scenario data configmap..."
kubectl -n demo-namespace --context kind-rtf-workload \
  create configmap rtf-scenario-config \
  --from-file=rtf_data/scenario.yaml

sleep 2

echo "Running scenario job..."
kubectl -n demo-namespace --context kind-rtf-workload \
  apply -f demo/scenario-job.yaml

echo "Follow job progress in k9s!"

#!/usr/bin/env bash
# Runs an RTF test plan using rtf run
#
# Usage: run-test-plan.sh <test_plan_path> [--vars <vars_file>] [--verbose]
#   test_plan_path  - path to the test plan file (required)
#   --vars          - path to a JSON file containing template variables (optional)
#   --verbose       - enable TRACE level logging (default is INFO)

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "Usage: run-test-plan.sh <test_plan_path> [--vars <vars_file>] [--verbose]"
  exit 1
fi

TEST_PLAN_PATH="$1"
shift

VERBOSE_FLAGS="-v"  # Default to INFO level
VARS_ARGS=()

while [[ $# -gt 0 ]]; do
  case $1 in
    --vars)
      if [[ -n "${2:-}" ]]; then
        VARS_ARGS=("--vars" "$2")
      fi
      shift 2
      ;;
    --verbose)
      VERBOSE_FLAGS="-vvv"  # TRACE level
      shift
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

exec rtf ${VERBOSE_FLAGS} run "${VARS_ARGS[@]+"${VARS_ARGS[@]}"}" "${TEST_PLAN_PATH}"

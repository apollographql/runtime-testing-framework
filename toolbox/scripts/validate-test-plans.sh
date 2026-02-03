#!/usr/bin/env bash
# Validates RTF test plans using rtf template --check
#
# Usage: validate-test-plans.sh <directories> <test plan name> <variables name>
#   directories        - comma-separated list of directories to search
#   test plan name     - filename pattern for test plan files
#   variables name     - filename pattern for template variables files

set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "Usage: validate-test-plans.sh <directories> <test plan name> <variables name>"
  exit 1
fi

DIRECTORIES_INPUT="$1"
TEST_PLAN_NAME="$2"
VARIABLES_NAME="$3"

PASS_COUNT=0
FAIL_COUNT=0
FAILED_PLANS=()

IFS=',' read -ra DIRECTORIES <<< "${DIRECTORIES_INPUT}"

for DIR in "${DIRECTORIES[@]}"; do
  DIR=$(echo "$DIR" | xargs)

  if [[ ! -d "${DIR}" ]]; then
    echo "Error: Directory does not exist: ${DIR}"
    exit 1
  fi

  while IFS= read -r -d '' TEST_PLAN; do
    PLAN_DIR=$(dirname "$TEST_PLAN")
    VARS_FILE="${PLAN_DIR}/${VARIABLES_NAME}"

    if [[ ! -f "$VARS_FILE" ]]; then
      echo "Error: Missing ${VARIABLES_NAME} for ${TEST_PLAN}"
      exit 1
    fi

    echo "=== Validating: ${TEST_PLAN} ==="

    if rtf template "${TEST_PLAN}" --vars "${VARS_FILE}" --check > /dev/null; then
      echo "PASS"
      ((PASS_COUNT++)) || true
    else
      echo "FAIL"
      ((FAIL_COUNT++)) || true
      FAILED_PLANS+=("${TEST_PLAN}")
    fi

    echo ""
  done < <(find "${DIR}" -name "${TEST_PLAN_NAME}" -print0)
done

echo "========================================"
echo "Results: ${PASS_COUNT} passed, ${FAIL_COUNT} failed"
echo "========================================"

if [[ ${FAIL_COUNT} -gt 0 ]]; then
  echo ""
  echo "Failed test plans:"
  for PLAN in "${FAILED_PLANS[@]}"; do
    echo "  - ${PLAN}"
  done
  exit 1
fi

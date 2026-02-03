#!/usr/bin/env bash
# This script is intended for use inside of the `rtf-test-custom-providers` GitHub Action
# found in the release-tooling repository: https://github.com/apollographql/release-tooling
#
# Usage: test-custom-providers.sh <search dirs> <provider name>
#   search dirs    - comma-separated list of directories to search
#   provider name  - filename pattern for custom provider files

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: test-custom-providers.sh <search dirs> <provider filename>"
  exit 1
fi

SEARCH_DIRS="$1"
PROVIDER_FILENAME="$2"

RTF_CMD="rtf custom-provider test --error-on-empty"
PASS_COUNT=0
FAIL_COUNT=0

# Split the requested top level search directories by comma
IFS=',' read -ra DIRS <<< "$SEARCH_DIRS"

for DIR in "${DIRS[@]}"; do
  DIR=$(echo "$DIR" | xargs)

  if [ ! -d "$DIR" ]; then
    echo "ERROR: Directory '$DIR' does not exist"
    exit 1
  fi

  while IFS= read -r -d '' PROVIDER; do
    echo ":: Running tests for $PROVIDER..."
    if $RTF_CMD "$PROVIDER"; then
      ((PASS_COUNT++)) || true
    else
      ((FAIL_COUNT++)) || true
    fi

    echo ""
  done < <(find "$DIR" -path "*/$PROVIDER_FILENAME" -print0)
done

echo "========================================"
echo "Results: $PASS_COUNT passed, $FAIL_COUNT failed"
echo "========================================"

if [ $FAIL_COUNT -gt 0 ]; then
  exit 1
fi

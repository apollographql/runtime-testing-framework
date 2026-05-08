#!/usr/bin/env bash
# Check that README.md is in sync with values.yaml and README.md.gotmpl
# by regenerating it in a temp directory and comparing with the committed version.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CHART_DIR="$PROJECT_ROOT/crates/rep-orchestrator/helm/rep-orchestrator"
README_FILE="$CHART_DIR/README.md"
DPRINT_CONFIG="$PROJECT_ROOT/dprint.json"

# Check if helm-docs is available
if ! command -v helm-docs &> /dev/null; then
    echo "ERROR: helm-docs is not installed"
    exit 1
fi

# Check if dprint is available
if ! command -v dprint &> /dev/null; then
    echo "ERROR: dprint is not installed"
    exit 1
fi

# Check if README.md exists
if [[ ! -f "$README_FILE" ]]; then
    echo "ERROR: README.md not found at $README_FILE"
    exit 1
fi

# Create a temporary directory
TEMP_DIR=$(mktemp -d)
trap "rm -rf $TEMP_DIR" EXIT

# Copy the chart to the temp directory
cp -r "$CHART_DIR" "$TEMP_DIR/chart"

# Run helm-docs on the temp copy
helm-docs -c "$TEMP_DIR/chart" > /dev/null 2>&1 || {
    echo "ERROR: helm-docs failed"
    exit 1
}

# Format the generated README with dprint to match the committed version's formatting.
# Use --stdin because dprint applies its config's includes/excludes (rooted at the project)
# and won't format files outside the project tree.
FORMATTED_README="$TEMP_DIR/chart/README.formatted.md"
dprint fmt --stdin README.md --config "$DPRINT_CONFIG" \
    < "$TEMP_DIR/chart/README.md" > "$FORMATTED_README" || {
    echo "ERROR: dprint fmt failed"
    exit 1
}
mv "$FORMATTED_README" "$TEMP_DIR/chart/README.md"

# Compare the generated README.md with the committed version
if diff -u "$README_FILE" "$TEMP_DIR/chart/README.md" > /dev/null 2>&1; then
    echo "✓ README.md is in sync with values.yaml"
    exit 0
else
    echo "ERROR: README.md is out of sync with values.yaml"
    echo ""
    echo "Run 'mise run helm-docs' to regenerate README.md"
    echo ""
    diff -u "$README_FILE" "$TEMP_DIR/chart/README.md" || true
    exit 1
fi

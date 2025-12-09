#!/usr/bin/env bash
# Check that all documentation files have a valid diataxis type comment on the first line.
# Valid types: tutorial, howto, reference, explanation
# Format: <!-- diataxis-type: <type> -->

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCS_DIR="${SCRIPT_DIR}/../docs/src"
VALID_TYPES="tutorial|howto|reference|explanation"
ERRORS=0

# Files to skip (no type declaration required)
SKIP_FILES=("SUMMARY.md")

should_skip() {
    local filename="$1"
    for skip in "${SKIP_FILES[@]}"; do
        if [[ "$filename" == "$skip" ]]; then
            return 0
        fi
    done
    return 1
}

check_file() {
    local file="$1"
    local rel_path="${file#$DOCS_DIR/}"
    local filename
    filename="$(basename "$file")"

    # Skip files that don't need type declaration
    if should_skip "$filename"; then
        return 0
    fi

    # Read the first line
    local first_line
    first_line=$(head -1 "$file")

    # Check for diataxis-type comment format
    if ! echo "$first_line" | grep -qE "^<!--\s*diataxis-type:\s*($VALID_TYPES)\s*-->$"; then
        # Provide helpful error message
        if echo "$first_line" | grep -q "diataxis-type"; then
            echo "ERROR: $rel_path - Invalid diataxis-type format. Expected: <!-- diataxis-type: <type> -->"
        else
            echo "ERROR: $rel_path - Missing diataxis-type comment on first line"
        fi
        return 1
    fi

    return 0
}

echo "Checking documentation diataxis-type declarations..."
echo ""

# Find all markdown files in docs/src
while IFS= read -r -d '' file; do
    if ! check_file "$file"; then
        ERRORS=$((ERRORS + 1))
    fi
done < <(find "$DOCS_DIR" -name "*.md" -type f -print0)

echo ""
if [[ $ERRORS -gt 0 ]]; then
    echo "Found $ERRORS diataxis-type error(s)"
    exit 1
fi

echo "All documentation files have valid diataxis-type declarations"

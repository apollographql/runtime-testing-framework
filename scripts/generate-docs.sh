#!/usr/bin/env bash
# Use the JSON schema of the file provider types to generate the user docs for file providers.

REPO_ROOT="$(git rev-parse --show-toplevel)"
DOCS_PAGE="$REPO_ROOT/docs/src/reference/framework/file-providers.md"

echo ">> Generating file provider docs from JSON schema..."
cargo run --bin rtf-docgen > "$DOCS_PAGE"

echo ">> Formatting markdown..."
mise format-markdown

echo ">> Done"

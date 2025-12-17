#!/usr/bin/env bash
# Use the JSON output from rustdoc to generate the user docs for file providers.
#   -> Rustdoc's support for outputting JSON is still in development, so there
#      is a good chance that the format being parsed here will change over time.
#      If that happens, we need to update the version of rustdoc-types in the
#      rtf-docgen CLI to match the version being printed by this script.

REPO_ROOT="$(git rev-parse --show-toplevel)"
DOCS_PAGE="$REPO_ROOT/docs/src/reference/framework/file-providers.md"
RAW="$REPO_ROOT/target/doc/rtf_config.json"

echo ">> Generating rustdoc JSON for file provider types..."
RUSTC_BOOTSTRAP=1 RUSTDOCFLAGS="-Z unstable-options --output-format json" \
  cargo doc --no-deps --document-private-items

echo ">> Current doc format version is 0.$(jq '.format_version' < $RAW). This must match rustdoc-types in the CLI crate."

echo ">> Processing rustdoc JSON output..."
{
  echo "<!-- diataxis-type: reference -->"
  echo
  cargo run --bin rtf-docgen "$RAW"
} > "$DOCS_PAGE"

echo ">> Formatting markdown..."
mise format-markdown

echo ">> Done"

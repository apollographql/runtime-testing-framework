#!/usr/bin/env bash
# Use the JSON output from rustdoc to generate the user docs for file providers.
#   -> Rustdoc's support for outputting JSON is still in development, so there
#      is a good chance that the format being parsed here will change over time.
#      As and when this happens we will need to update the script to handle any
#      breaking changes to the format.

REPO_ROOT="$(git rev-parse --show-toplevel)"
DOCS_PAGE="$REPO_ROOT/docs/src/framework/file-providers.md"
RAW="$REPO_ROOT/target/doc/rtf_config.json"

function lookup {
  jq ".index.[\"$1\"]" <"$RAW"
}

function toc {
  FP="$(jq '.index | .[] | select(.name == "FileProvider")' <"$RAW")"
  VARIANTS="$(echo "$FP" | jq -r '.inner.enum.variants | @sh')"

  echo "Available file providers:"

  for variant in $VARIANTS; do
    VARIANT="$(lookup "$variant")"
    INNER="$(lookup "$(echo "$VARIANT" | jq -r '.inner.variant.kind.tuple[0]')")"
    TRUE_INNER="$(lookup "$(echo "$INNER" | jq -r '.inner.struct_field.resolved_path.id')")"
    NAME="$(echo "$TRUE_INNER" | jq -r '.docs' | head -n1 | sed 's/# //g')"

    KEBAB_NAME="$(
      echo "$NAME" | 
        tr '[:upper:]' '[:lower:]' |
        tr ' ' '-'
    )"
    echo "- [$NAME](#$KEBAB_NAME)"
  done
  printf "\n\n"
}

function process_output {
  FP="$(jq '.index | .[] | select(.name == "FileProvider")' <"$RAW")"
  VARIANTS="$(echo "$FP" | jq -r '.inner.enum.variants | @sh')"


  for variant in $VARIANTS; do
    VARIANT="$(lookup "$variant")"
    INNER="$(lookup "$(echo "$VARIANT" | jq -r '.inner.variant.kind.tuple[0]')")"
    TRUE_INNER="$(lookup "$(echo "$INNER" | jq -r '.inner.struct_field.resolved_path.id')")"
    DOCS="$(echo "$TRUE_INNER" | jq -r '.docs')"
    NAME="$(echo "$TRUE_INNER" | jq -r '.name')"

    # Resolved values is a unit struct without fields
    if [ "$NAME" = "ResolvedValues" ]; then
      printf "#%s\n" "$DOCS"
      continue
    fi

    FIELDS="$(echo "$TRUE_INNER" | jq -r '.inner.struct.kind.plain.fields | @sh')"

    printf "#%s\n\n" "$DOCS"
    echo "### Fields"

    for field in $FIELDS; do
      FIELD="$(lookup "$field")"
      FIELD_NAME="$(echo "$FIELD" | jq -r '.name')"
      echo "#### \`$FIELD_NAME\`"
      echo "$FIELD" | jq -r '.docs'
      echo ""
    done

    printf "\n"
  done
}

echo ">> Generating rustdoc JSON for file provider types..."
RUSTC_BOOTSTRAP=1 RUSTDOCFLAGS="-Z unstable-options --output-format json" \
  cargo doc --no-deps --document-private-items

echo ">> Processing rustdoc JSON output..."
printf "# File Providers\n" > "$DOCS_PAGE"
toc >> "$DOCS_PAGE"
process_output >> "$DOCS_PAGE"

echo ">> Formatting markdown..."
mise format-markdown

echo ">> Done"

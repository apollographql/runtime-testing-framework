#!/usr/bin/bash

# TODO:
#   - support using gq-op-gen instead of a canned file

TEST_DIR="${OUTDIR:-$(pwd)}/tests"
RESULTS_DIR="$TEST_DIR/results"
CANNED="$TEST_DIR/requests.canned"
CANNED_TMP="$(mktemp /tmp/router.XXXXXX)"
N_OPS="$(wc -l "$CANNED_OPS_FILE")"
N_COPIES="$(( RPS * DURATION / N_OPS ))"

# rewrite our canned request data into vegeta format
while read -r req; do
  jq -nc \
    --arg body "$(echo "$req" | base64 -w 0 -i)" \
    '{
      "body": $body,
      "header": { "Content-type": ["application/json"] },
      "method": "POST",
      "url": "http://127.0.0.1:4000/"
    }' >> "$CANNED"
done <"$CANNED_OPS_FILE"

yes "$CANNED" |
  head -n "$(( N_COPIES>1 ? N_COPIES : 1 ))" |
  xargs cat |
  shuf --random-source=<(yes "$RANDOM_SEED") > "$CANNED_TMP"

mv "$CANNED_TMP" "$CANNED"

vegeta attack \
  -lazy \
  -format=json \
  -max-connections=500000 \
  -h2c \
  -timeout 60s \
  -output "$RESULTS_DIR/perf.$$.vegeta" \
  -rate="$RPS/s" \
  -duration="${DURATION_SECS}s" <"$CANNED"

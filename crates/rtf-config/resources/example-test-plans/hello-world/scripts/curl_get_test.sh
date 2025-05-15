#!/usr/bin/env sh

runTest() {
  PASSING="true"

  while read -r case; do
    endpoint="$(echo "$case" | jq -r '.endpoint')"
    expected_status_code="$(echo "$case" | jq -r '.status')"
    expected_resp="$(echo "$case" | jq -r '.resp')"

    status_code="$(
      curl -s -o response.txt -w "%{response_code}" -u "$AUTH_HEADER" "$SERVER_URL/$endpoint"
    )"

    if [ "$status_code" != "$expected_status_code" ]; then
      echo ">>> UNEXPECTED STATUS CODE"
      echo "    case: $case"
      echo "    expected $expected_status_code but got $status_code"
      PASSING="false"
    fi

    if [ "$(cat response.txt)" != "$expected_resp" ]; then
      echo ">>> UNEXPECTED STATUS CODE"
      echo "    case: $case"
      echo "    expected $expected_status_code but got $status_code"
      PASSING="false"
    fi

    rm response.txt
  done < "$REQUESTS_FILE"

  if [ "$PASSING" = "false" ]; then
    echo "tests failed" > results/test-result.txt
    exit 1
  else
    echo "tests passed" > results/test-result.txt
    exit 0
  fi
}

mkdir results
runTest > results/test-output.txt

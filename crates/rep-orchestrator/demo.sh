#!/usr/bin/env bash
# Demonstration of the expected user flow for triggering a test run and polling for the result

TP_PATH="./resources/test-plans/valid/minimal/test-plan.yaml"
# ../../../rtf-morgue/test-plans/router-validation/rep-compatible/test-plan.yaml

echo ":: Triggering test run using $TP_PATH"
RUN_ID="$(
  curl -s localhost:8035/test-run/trigger \
    -H "Content-Type: application/json" \
    -d "$(rtf rep prepare "$TP_PATH")" |
  jq -r '.id'
)"
echo ":: Run ID: $RUN_ID"

echo -e "\n:: Wait for run to complete\n"

STATUS="INIT"
PROGRESS=""

while [ "$STATUS" != "SUCCESSFUL" ]; do
  RESP="$(curl -s "localhost:8035/test-run/$RUN_ID/status")"
  N_EXECUTIONS="$(echo "$RESP" | jq -r '.executions | length')"

  if [ "$N_EXECUTIONS" = "1" ]; then
    STATUS="$(echo "$RESP" | jq -r '.executions[0].status_history[0].status')"
    MSG="$(echo "$RESP" | jq -r '.executions[0].status_history[0].message')"
    AT="$(echo "$RESP" | jq -r '.executions[0].status_history[0].updated_at')"
  else
    STATUS="$(echo "$RESP" | jq -r '.current_status')"
    MSG="$(echo "$RESP" | jq -r '.status_history[0].message')"
    AT="$(echo "$RESP" | jq -r '.updated_at')"
  fi

  NEW="$AT $STATUS :: $MSG"
  if [ "$NEW" != "$PROGRESS" ]; then
    echo "$NEW"
    PROGRESS="$NEW"
  fi

  sleep 1
done

echo -e "\n:: Fetching Execution ID of the test execution"
EX_ID="$(
  curl -s "localhost:8035/test-run/$RUN_ID/status" |
    jq -r '.executions[0].id'
)"
echo -e ":: Execution ID: $EX_ID\n"

echo ":: Pulling execution log:"
curl -s "localhost:8035/test-execution/$EX_ID/log.txt"

echo -e "\n:: Pulling execution output:"
curl -sL "localhost:8035/test-execution/$EX_ID/output.zip" > output.zip
unzip output.zip

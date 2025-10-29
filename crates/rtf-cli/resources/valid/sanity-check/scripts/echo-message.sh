#!/usr/bin/env sh

echo "${STAGE} :: ${MESSAGE}" >> "$OUTDIR/$STAGE.txt"

if [ -n "$FROM_SETUP" ]; then
  echo "${STAGE} :: ${FROM_SETUP}" >> "$OUTDIR/$STAGE.txt"
fi

if [ "${STAGE}" = "env-setup" ]; then
  echo '{ "setup_output": "output from setup" }' > "$RTF_OUTPUT"
fi

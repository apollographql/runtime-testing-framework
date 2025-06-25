#!/usr/bin/env sh

echo ">>> Hello from ${STAGE}!"
echo "${STAGE} :: ${MESSAGE}${SUBJECT}" >> "$OUTDIR/combined-output.txt"

if [ "${STAGE}" = "env-setup" ]; then
  echo '{}' > "$RTF_OUTPUT"
fi

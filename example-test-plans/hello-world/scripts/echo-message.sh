#!/usr/bin/env sh

echo "${STAGE} :: ${MESSAGE}${SUBJECT}" | tee "$OUTDIR/combined-output.txt"

if [ "${STAGE}" = "env-setup" ]; then
  echo '{}' > "$RTF_OUTPUT"
fi

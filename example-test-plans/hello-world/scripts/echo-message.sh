#!/usr/bin/env sh

echo "${STAGE} :: ${MESSAGE}${SUBJECT}" | tee -a "$OUTDIR/combined-output.txt"

if [ "${STAGE}" = "env-setup" ]; then
  echo '{}' > "$RTF_OUTPUT"
fi

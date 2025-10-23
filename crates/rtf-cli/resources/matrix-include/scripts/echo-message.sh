#!/usr/bin/env sh

echo "${STAGE} :: ${MESSAGE}${SUBJECT}"

if [ "${STAGE}" = "env-setup" ]; then
  echo '{}' > "$RTF_OUTPUT"
fi

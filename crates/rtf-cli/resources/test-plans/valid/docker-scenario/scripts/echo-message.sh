#!/usr/bin/env sh

echo "${STAGE} :: ${MESSAGE}"
echo "${STAGE} :: ${MESSAGE}" >> "$OUTDIR/$STAGE.txt"

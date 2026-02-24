#!/usr/bin/env sh

echo "${STAGE} :: ${MESSAGE}" | tee -a "$OUTDIR/$STAGE.txt"

#!/usr/bin/env sh

echo "Hello, $SUBJECT!" | tee "$RTF_OUTPUT"
cat "$README" | tee -a "$RTF_OUTPUT"

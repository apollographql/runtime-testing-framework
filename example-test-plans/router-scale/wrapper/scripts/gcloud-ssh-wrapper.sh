#!/bin/bash
host="$1"
shift

exec gcloud compute ssh \
  --project="router-performance" \
  --zone="us-central1-b" \
  --impersonate-service-account="sa-router-performance@router-performance.iam.gserviceaccount.com" \
  "$host" \
  -- \
  "$@"

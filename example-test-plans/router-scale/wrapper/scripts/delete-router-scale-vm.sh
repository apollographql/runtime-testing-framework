#! /usr/bin/env bash

# This script will delete the VM every time RTF runs router-scale
# We will add the option to keep the VM and cleanup the data in a future ticket

shopt -s expand_aliases

alias GCLOUD_SA="gcloud --impersonate-service-account=sa-router-performance@router-performance.iam.gserviceaccount.com"

# Used to filter out annoying impersonation warning message
FILTER_OUT="WARNING: This command is using service account impersonation."

# Used to log output to text file
OUTDIR="${OUTDIR:-$(pwd)}"
LOG_FILE="$OUTDIR/vm-log.txt"

###
# Delete an instance
###
function delete_vm {
    echo "Deleting test system: $1..."
    GCLOUD_SA compute instances delete "$1" \
        --quiet \
        --project=router-performance \
        --zone=us-central1-b \
        2> >(grep -v "${FILTER_OUT}")
    echo "Test system: $1 deleted."
}

if [ -z "$1" ]; then
    echo "Usage: $0 <vm-name>"
    exit 1
fi
delete_vm "$1" >> $LOG_FILE

# Echo empty output so RTF environment teardown succeeds
echo {}

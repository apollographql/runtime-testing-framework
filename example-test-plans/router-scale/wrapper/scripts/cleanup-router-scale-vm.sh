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
# Cleanup after a router-scale test
###
function cleanup_vm {
    echo "Cleaning up test system: $1..."
    echo "WIP - NO CLEANUP STEPS DEFINED"
    echo "Test system: $1 cleaned up."
}

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

if [ $# -ne 2 ]; then
    echo "Usage: $0 <vm-name> <delete-vm>"
    exit 1
fi

VM_NAME="$1"
DELETE_VM="$2"

if [ "$DELETE_VM" != "true" ]; then
    cleanup_vm $VM_NAME >> $LOG_FILE
else
    delete_vm $VM_NAME >> $LOG_FILE
fi

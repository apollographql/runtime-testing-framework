#! /usr/bin/env bash

# If the the delete_vm argument is set to "true" then this script will delete the VM
# Any other valud for delete_vm will instead leave the VM and cleanup any sensitive
# files from the test run(s)

BASE_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
if [ -z "$RTF_DIR" ]; then
    echo "Error: RTF_DIR environment variable is not set."
    exit 1
fi

shopt -s expand_aliases

alias GCLOUD_SA="gcloud --impersonate-service-account=sa-router-performance@router-performance.iam.gserviceaccount.com"

# Used to filter out annoying impersonation warning message
FILTER_OUT="WARNING: This command is using service account impersonation."

# Used to log output to text file
OUTDIR="${OUTDIR:-$(pwd)}"
LOG_FILE="$OUTDIR/vm-log.txt"

# Used to find the gcloud ssh wrapped script
# This is required so that these can be referenced by RTF
GCLOUD_SSH_WRAPPER="${GCLOUD_SSH_WRAPPER:-$BASE_DIR/gcloud-ssh-wrapper.sh}"

###
# Cleanup after a router-scale test
###
function cleanup_vm {
    echo "Cleaning up test system: $1..."
    
    # Set the directories that router-scale output will be written to
    OUTDIR="./output"
    
    # Clean up all the sensitive data
    bash $GCLOUD_SSH_WRAPPER $1 rm -f "values.json" 2> >(grep -v "${FILTER_OUT}") >> $LOG_FILE
    bash $GCLOUD_SSH_WRAPPER $1 rm -f "$OUTDIR/canned_ops.json" 2> >(grep -v "${FILTER_OUT}") >> $LOG_FILE
    bash $GCLOUD_SSH_WRAPPER $1 rm -f "$OUTDIR/license.jwt" 2> >(grep -v "${FILTER_OUT}") >> $LOG_FILE
    bash $GCLOUD_SSH_WRAPPER $1 rm -f "$OUTDIR/router-config.yaml" 2> >(grep -v "${FILTER_OUT}") >> $LOG_FILE
    bash $GCLOUD_SSH_WRAPPER $1 rm -f "$OUTDIR/supergraph.graphql" 2> >(grep -v "${FILTER_OUT}") >> $LOG_FILE
    bash $GCLOUD_SSH_WRAPPER $1 rm -rf "$OUTDIR/subgraphs/" 2> >(grep -v "${FILTER_OUT}") >> $LOG_FILE
    bash $GCLOUD_SSH_WRAPPER $1 rm -f "$OUTDIR/tests/requests.canned" 2> >(grep -v "${FILTER_OUT}") >> $LOG_FILE
    bash $GCLOUD_SSH_WRAPPER $1 rm -rf "$OUTDIR/tests/results/" 2> >(grep -v "${FILTER_OUT}") >> $LOG_FILE

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

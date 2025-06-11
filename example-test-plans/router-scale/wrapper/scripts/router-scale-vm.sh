#! /usr/bin/env bash

shopt -s expand_aliases

BASE_DIR=$(dirname $0)

###
# Check we have the required dependencies
####
source "${BASE_DIR}/helpers/local-dependencies.sh" && check_dependencies || exit 2

alias GCLOUD_SA="gcloud --impersonate-service-account=sa-router-performance@router-performance.iam.gserviceaccount.com"

# Used to filter out annoying impersonation warning message
FILTER_OUT="WARNING: This command is using service account impersonation."

# Used to log output to text file
OUTDIR="${OUTDIR:-$(pwd)}"
LOG_FILE="$OUTDIR/vm-log.txt"

###
# Create an instance.
#
# Try to boot from an existing master image. If there is no master image boot from scratch and create a new master image for future use.
###
function create_vm {
    echo "Creating test system: $1..."
    if GCLOUD_SA compute machine-images describe router-perf-master --project router-performance > /dev/null 2> >(grep -v "${FILTER_OUT}"); then
        # We have a master, clone it
        GCLOUD_SA beta compute instances create "$1" \
            --project=router-performance \
            --zone=us-central1-b \
            --machine-type=e2-highcpu-32 \
            --network-interface=network-tier=PREMIUM,stack-type=IPV4_ONLY,subnet=default \
            --maintenance-policy=MIGRATE \
            --provisioning-model=STANDARD \
            --instance-termination-action=DELETE \
            --max-run-duration=10800s \
            --min-cpu-platform=Automatic \
            --no-shielded-secure-boot \
            --shielded-vtpm \
            --shielded-integrity-monitoring \
            --labels="goog-ec-src=vm_add-gcloud,perf-test=$1" \
            --reservation-affinity=any \
            --source-machine-image=router-perf-master \
            --service-account=sa-router-performance@router-performance.iam.gserviceaccount.com \
            --scopes=https://www.googleapis.com/auth/devstorage.read_only,https://www.googleapis.com/auth/logging.write,https://www.googleapis.com/auth/monitoring.write,https://www.googleapis.com/auth/servicecontrol,https://www.googleapis.com/auth/service.management.readonly,https://www.googleapis.com/auth/trace.append \
            2> >(grep -v "${FILTER_OUT}") \
            > /dev/null
        echo "Test system: $1 created."
    else
        # We need to create a master
        echo "Creating a master is not supported for now."
    fi
}

###
# List instance
###
function list_vm {
    GCLOUD_SA compute instances list \
        --filter="zone ~ us AND labels.perf-test:*" \
        --project router-performance \
        2> >(grep -v "${FILTER_OUT}")
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

case "$1" in
    create)
        if [ -z "$2" ]; then
            echo "Usage: $0 create <vm-name>"
            exit 1
        fi
        create_vm "$2" >> $LOG_FILE

        # Echo empty output so RTF environment setup succeeds
        echo {}
        ;;
    delete)
        if [ -z "$2" ]; then
            echo "Usage: $0 delete <vm-name>"
            exit 1
        fi
        delete_vm "$2" >> $LOG_FILE

        # Echo empty output so RTF environment teardown succeeds
        echo {}
        ;;
    list)
        list_vm
        ;;
    *)
        echo "Usage: $0 {create <vm-name>|delete <vm-name>|list}"
        exit 1
        ;;
esac

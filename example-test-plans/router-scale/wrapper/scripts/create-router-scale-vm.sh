#! /usr/bin/env bash

shopt -s expand_aliases

BASE_DIR=$(dirname $0)

alias GCLOUD_SA="gcloud --impersonate-service-account=sa-router-performance@router-performance.iam.gserviceaccount.com"

# Used to filter out annoying impersonation warning message
FILTER_OUT="WARNING: This command is using service account impersonation."

# Used to log output to text file
OUTDIR="${OUTDIR:-$(pwd)}"
LOG_FILE="$OUTDIR/vm-log.txt"

###
# Dependencies required to run local router-scale scripts:
#
#  It's important to keep this list up to date
###
dependencies=("gcloud" "rsync")

###
# report
#
# Report any missing commands.
#
# $*: Missing commands
###
function report {
    printf "You need the following components to use this command:\n"
    printf "\t %s\n" "${dependencies[@]}"
    printf "You are missing %s\n" "$*"
    printf "Some of these components can be installed from homebrew\n"
    return 1
}

###
# Check we have the required dependencies
####
function check_dependencies {
    # Before checking dependencies make sure we are running bash 5 at least
    if ! bash --version | grep -q 'version 5\.'; then
        printf "Bash v5 is required to run these scripts.\n"
        printf "Note: If this is the future and we now have Bash 6, update this check\n"
        printf "Note: You can easily install Bash 5 from homebrew\n"
        return 1
    fi
    missing=()
    for dependency in "${dependencies[@]}"; do
        if ! which "${dependency}" > /dev/null 2>&1; then
            missing+=("${dependency}")
        fi
    done
    if (( ${#missing[@]} != 0 )); then
        report "${missing[@]}"
    fi
}

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

if [ -z "$1" ]; then
    echo "Usage: $0 <vm-name>"
    exit 1
fi
check_dependencies || exit 2
create_vm "$1" >> $LOG_FILE

# Echo empty output so RTF environment setup succeeds
echo {} > "$RTF_OUTPUT"

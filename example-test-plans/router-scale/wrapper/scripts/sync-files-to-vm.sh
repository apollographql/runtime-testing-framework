#! /usr/bin/env bash

BASE_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
if [ -z "$RTF_DIR" ]; then
    echo "Error: RTF_DIR environment variable is not set."
    exit 1
fi

# Used to log output to text file
OUTDIR="${OUTDIR:-$(pwd)}"
LOG_FILE="$OUTDIR/vm-log.txt"

# Used to filter out annoying impersonation warning message
FILTER_OUT="WARNING: This command is using service account impersonation."

# Used to find the gcloud ssh wrapped and install rtf script
# This is required so that these can be referenced by RTF
GCLOUD_SSH_WRAPPER="${GCLOUD_SSH_WRAPPER:-$BASE_DIR/gcloud-ssh-wrapper.sh}"
INSTALL_RTF="${INSTALL_RTF:-$BASE_DIR/install-rtf.sh}"
BOOTSTRAP_ENV="${BOOTSTRAP_ENV:-$BASE_DIR/bootstrap-env.sh}"

function rsync_to_vm {
    # $1 is the target VM name (it has to be the first argument since the ssh script expects the first argument to be the vm name)
    # $2 is the source dir
    # $3 is the target dir
    echo "Copying $2 to $1:$3..."
    
    rsync -e "bash $GCLOUD_SSH_WRAPPER" --compress --recursive --times "$2" "$1:$3"\
        2> >(grep -v "${FILTER_OUT}")
}

if [ -z "$1" ]; then
    echo "Usage: $0 <vm-name>"
    exit 1
fi

# Check the VM is ready and can be ssh'd to
SSH_CMD="bash $GCLOUD_SSH_WRAPPER $1 ls"
LOGIN_SUCCESS=false
i=0
while (( i < 20 )); do
    echo "Attempting to ssh to $1 vm..." >> $LOG_FILE
    if $SSH_CMD 2> >(grep -v "${FILTER_OUT}") >> $LOG_FILE; then
        echo "Ssh to $1 vm success" >> $LOG_FILE
        LOGIN_SUCCESS=true
        break
    fi
    echo "Attempt $i to ssh to $1 vm failed. Retrying..." >> $LOG_FILE
    sleep 10
    ((i++))
done

if [ "$LOGIN_SUCCESS" = false ]; then
    echo "Failed to ssh to $1 vm after multiple attempts." >> $LOG_FILE
    exit 1
fi


# Copy files required to build RTF over to the VM
rsync_to_vm $1 $RTF_DIR/Cargo.toml ./rtf/ >> $LOG_FILE
rsync_to_vm $1 $RTF_DIR/Cargo.lock ./rtf/ >> $LOG_FILE
rsync_to_vm $1 $RTF_DIR/crates/ ./rtf/crates/ >> $LOG_FILE
# The test plan we're going to run
rsync_to_vm $1 $RTF_DIR/$2/ ./test-data/ >> $LOG_FILE

# Copy shell script to install rtf to VM
rsync_to_vm $1 $BOOTSTRAP_ENV ./ >> $LOG_FILE
rsync_to_vm $1 $INSTALL_RTF ./ >> $LOG_FILE

# Bootstrap the environment
bash $GCLOUD_SSH_WRAPPER $1 bash -i bootstrap-env.sh 2> >(grep -v "${FILTER_OUT}") >> $LOG_FILE

# Run the rtf script
echo "Install RTF and run on the VM..." >> $LOG_FILE
bash $GCLOUD_SSH_WRAPPER $1 bash -i install-rtf.sh "$APOLLO_KEY" 2> >(grep -v "${FILTER_OUT}") >> $LOG_FILE

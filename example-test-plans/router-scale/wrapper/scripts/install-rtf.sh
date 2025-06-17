#!/bin/bash

# This script will be run on the VM. It installs rust and then runs rtf
# We need to install rust since the router-scale VM only has rust installed for the router user
# This was the easiest way to get started with running rtf on the router-scale VM.
# We may run RTF under a different user in a future ticket.

if ! which cargo >/dev/null 2>&1; then
    echo "cargo not found, installing rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    . "$HOME/.cargo/env"
else
    echo "cargo is already installed."
fi

#  Run the sanity check
echo "Running rtf santity check..."
cargo run --manifest-path ./rtf/Cargo.toml -- run rtf/crates/rtf-cli/resources/sanity-check/test-plan.yaml
echo "------ env-setup output ------"
cat output/env-setup.txt
echo "------ scenario output ------"
cat output/scenario.txt
echo "------ env-teardown output ------"
cat output/env-teardown.txt
echo "Sanity check complete"
echo "Removing RTF output dir..."
rm -r output/

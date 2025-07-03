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

export APOLLO_SUDO="true"
export APOLLO_KEY="$1"

echo "Running test plan..."
cargo build --manifest-path ./rtf/Cargo.toml
cargo run --manifest-path ./rtf/Cargo.toml -- run test-data/test-plan.yaml --values "values.json" 2>&1

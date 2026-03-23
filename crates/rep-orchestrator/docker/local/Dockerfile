FROM rust:1.94.0

# aws-lc-rs (required by kube) needs cmake and clang
RUN apt-get update && apt-get install -y cmake clang curl netcat-openbsd \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-watch

# Source is mounted here at runtime (Tilt live_update or docker-compose volume mount)
RUN mkdir -p /runtime-testing-framework/crates/rep-orchestrator
WORKDIR /runtime-testing-framework/crates/rep-orchestrator

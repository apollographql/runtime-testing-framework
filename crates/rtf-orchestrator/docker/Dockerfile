FROM --platform=$BUILDPLATFORM rust:1.96.0 AS build

# aws-lc-rs (required by kube) needs cmake and clang
RUN apt-get update && apt-get install -y cmake clang && rm -rf /var/lib/apt/lists/*

WORKDIR /runtime-testing-framework

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
COPY ./crates ./crates

ARG GIT_SHA=unknown

# Cache mounts persist the cargo registry and compiled deps between builds so
# only rtf-orchestrator (and any locally-changed workspace crates) need recompiling.
# The binary is copied out to / because cache mounts are not available to later stages.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/runtime-testing-framework/target \
    cargo build --release --package rtf-orchestrator \
    && cp target/release/rtf-orchestrator /rtf-orchestrator


FROM --platform=$BUILDPLATFORM debian:trixie-slim

RUN apt-get update && apt-get install -y ca-certificates curl jq postgresql-client && rm -rf /var/lib/apt/lists/*

COPY --from=build /rtf-orchestrator .

ENTRYPOINT ["./rtf-orchestrator"]

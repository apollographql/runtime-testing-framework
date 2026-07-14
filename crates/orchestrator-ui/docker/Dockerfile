FROM --platform=$BUILDPLATFORM rust:1.96.0 AS build

# aws-lc-rs (pulled in via rustls/reqwest) needs cmake and clang
RUN apt-get update && apt-get install -y cmake clang && rm -rf /var/lib/apt/lists/*

WORKDIR /runtime-testing-framework

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
COPY ./crates ./crates

# Cache mounts persist the cargo registry and compiled deps between builds so only
# orchestrator-ui (and any locally-changed workspace crates) need recompiling.
# The HTML templates and vendored htmx are embedded into the binary at compile time,
# so no assets need to be copied into the runtime image.
# The binary is copied out to / because cache mounts are not available to later stages.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/runtime-testing-framework/target \
    cargo build --release --package orchestrator-ui \
    && cp target/release/orchestrator-ui /orchestrator-ui


FROM --platform=$BUILDPLATFORM debian:trixie-slim

RUN apt-get update && apt-get install -y ca-certificates curl && rm -rf /var/lib/apt/lists/*

COPY --from=build /orchestrator-ui .

ENTRYPOINT ["./orchestrator-ui"]

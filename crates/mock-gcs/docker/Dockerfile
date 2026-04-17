FROM --platform=$BUILDPLATFORM rust:1.94.1-bookworm AS build

# aws-lc-rs (required by kube) needs cmake and clang
RUN apt-get update && apt-get install -y cmake clang && rm -rf /var/lib/apt/lists/*

WORKDIR /runtime-testing-framework

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
COPY ./crates ./crates

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/runtime-testing-framework/target \
    cargo build --release --package mock-gcs \
    && cp target/release/mock-gcs /mock-gcs


FROM --platform=$BUILDPLATFORM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=build /mock-gcs .

ENTRYPOINT ["./mock-gcs"]

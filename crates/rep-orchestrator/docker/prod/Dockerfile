FROM --platform=$BUILDPLATFORM rust:1.94.0-bookworm AS build

# aws-lc-rs (required by kube) needs cmake and clang
RUN apt-get update && apt-get install -y cmake clang && rm -rf /var/lib/apt/lists/*

WORKDIR /runtime-testing-framework

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
COPY ./crates ./crates

RUN cargo build --release --package rep-orchestrator


FROM --platform=$BUILDPLATFORM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=build /runtime-testing-framework/target/release/rep-orchestrator .

ENTRYPOINT ["./rep-orchestrator"]

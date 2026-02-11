FROM rust:1.93.0 AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

RUN cargo build --release --bin rep-orchestrator

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
  && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/rep-orchestrator /usr/local/bin/rep-orchestrator

ENTRYPOINT ["rep-orchestrator"]

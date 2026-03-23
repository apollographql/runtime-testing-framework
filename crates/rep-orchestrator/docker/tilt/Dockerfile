FROM rust:1.94.0

RUN apt-get update && apt-get install -y cmake clang curl netcat-openbsd \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-watch

WORKDIR /runtime-testing-framework

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
COPY ./crates ./crates

WORKDIR /runtime-testing-framework/crates/rep-orchestrator

CMD ["cargo", "watch", "-i", "tests/**", "-x", "run"]

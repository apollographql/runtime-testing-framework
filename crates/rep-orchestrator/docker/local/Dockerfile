FROM rust:1.94.0

RUN apt-get update && apt-get install -y curl netcat-openbsd
RUN cargo install cargo-watch
RUN mkdir -p /runtime-testing-framework/crates/rep-orchestrator

WORKDIR /runtime-testing-framework/crates/rep-orchestrator

ENTRYPOINT ["cargo watch -i 'tests/**' -x run"]

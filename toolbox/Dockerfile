# Rust build image
FROM --platform=$BUILDPLATFORM rust:1.94.0 AS builder

WORKDIR /rtf

COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# Build Rust binaries
RUN cargo build --release --bin rtf --bin rep-orchestrator-cli


# Runtime image build
FROM --platform=$BUILDPLATFORM rust:1.94.0-slim

# https://kubernetes.io/releases/
ENV V_KUBECTL="v1.35.2"
# https://github.com/kubernetes/kompose/releases
ENV V_KOMPOSE="v1.38.0"

# Install tools available via apt
RUN apt-get update && apt-get install -y \
    curl \
    jq \
    docker.io \
    docker-compose \
    ca-certificates \
    iputils-ping \
    && rm -rf /var/lib/apt/lists/*

# https://kubernetes.io/docs/tasks/tools/install-kubectl-linux/#install-kubectl-binary-with-curl-on-linux
RUN curl -L "https://dl.k8s.io/release/$V_KUBECTL/bin/linux/$(dpkg --print-architecture)/kubectl" -o /usr/local/bin/kubectl \
    && chmod +x /usr/local/bin/kubectl

# https://kompose.io/installation/#github-release
RUN curl -L "https://github.com/kubernetes/kompose/releases/download/$V_KOMPOSE/kompose-linux-$(dpkg --print-architecture)" -o /usr/local/bin/kompose \
    && chmod +x /usr/local/bin/kompose


# Copy over Rust binaries from the build image
COPY --from=builder /rtf/target/release/rtf /usr/local/bin/rtf
COPY --from=builder /rtf/target/release/rep-orchestrator-cli /usr/local/bin/rep-orchestrator-cli


# Copy over scripts and add them to the PATH
COPY toolbox/scripts/ /toolbox/scripts/
RUN chmod +x /toolbox/scripts/* 2>/dev/null || true
ENV PATH="/toolbox/scripts:${PATH}"


CMD ["/bin/bash"]

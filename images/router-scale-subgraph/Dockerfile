FROM --platform=$BUILDPLATFORM golang:bookworm AS build

# Install git
RUN apt-get update && apt-get install -y git

# Copy the loadtesting directory
COPY loadtesting/ /app/loadtesting

# Set working directory
WORKDIR /app/loadtesting/cmd/subgraph

# Set up .netrc for Go to access private GitHub modules
RUN --mount=type=secret,id=GITHUB_TOKEN \
    echo "machine github.com login x-access-token password $(cat /run/secrets/GITHUB_TOKEN)" > /root/.netrc

# Build the subgraph binary
RUN go build

FROM --platform=$BUILDPLATFORM debian:bookworm-slim AS final

WORKDIR /app

# Copy the built binary from the previous stage
COPY --from=build /app/loadtesting/cmd/subgraph/subgraph .

# Set executable permissions (if needed)
RUN chmod +x ./subgraph

ENTRYPOINT ["./subgraph"]

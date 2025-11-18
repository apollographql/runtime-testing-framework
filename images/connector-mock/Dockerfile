FROM --platform=$BUILDPLATFORM rust:1.88 AS build

# create a new empty shell project
RUN USER=root cargo new --bin connector-mock
WORKDIR /connector-mock

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

# cache your dependencies
RUN cargo build --release
RUN rm src/*.rs
# copy over the actual source tree
COPY ./src ./src

# build for release
RUN rm -f target/release/deps/connector_mock* target/release/connector-mock
RUN cargo build --release


FROM --platform=$BUILDPLATFORM debian:bookworm-slim

COPY --from=build /connector-mock/target/release/connector-mock .

ENTRYPOINT ["./connector-mock"]

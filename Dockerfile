# syntax=docker/dockerfile:1
# Builds a static musl binary for the target platform and ships it in an
# empty image. Runs write to /data/output, so mount a volume there.
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY schema ./schema
COPY src ./src
RUN cargo build --release --locked && cp target/release/warmup-rs /warmup-rs

FROM scratch
COPY --from=build /warmup-rs /warmup-rs
WORKDIR /data
VOLUME ["/data/output"]
ENTRYPOINT ["/warmup-rs"]

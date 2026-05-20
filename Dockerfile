# syntax=docker/dockerfile:1.7

# Build stage --------------------------------------------------------------
FROM rust:1.91-slim-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin ethera-bundler \
    && cp /build/target/release/ethera-bundler /usr/local/bin/ethera-bundler

# Runtime stage ------------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --shell /usr/sbin/nologin bundler

COPY --from=builder /usr/local/bin/ethera-bundler /usr/local/bin/ethera-bundler

USER bundler
WORKDIR /home/bundler

EXPOSE 8082

ENTRYPOINT ["/usr/local/bin/ethera-bundler"]

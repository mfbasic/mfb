# Build and run the `mfb-repo` package-registry server with the S3 blob backend.
# The repository crate builds standalone (its own Cargo.lock), so the build
# context is just this directory.

FROM rust:1-bookworm AS builder

# Native build deps for the `s3` feature: aws-lc-sys (used by the AWS SDK's TLS
# stack) needs cmake + a C compiler + perl; rusqlite `bundled` compiles SQLite
# from source via cc.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential cmake perl clang libclang-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --features s3 --bin mfb-repo \
    && cp target/release/mfb-repo /usr/local/bin/mfb-repo

FROM debian:bookworm-slim AS runtime

# ca-certificates: TLS trust roots for reaching S3/Tigris over https.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /home/mfb mfb \
    && mkdir -p /data && chown mfb:mfb /data

COPY --from=builder /usr/local/bin/mfb-repo /usr/local/bin/mfb-repo
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

USER mfb
# Persist the metadata SQLite DB (and the server keypair it holds) across
# restarts; on Fly this is a mounted volume.
VOLUME ["/data"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]

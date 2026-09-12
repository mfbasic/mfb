# Build and run the `mfb-repo` package-registry server with the S3 blob backend.
#
# The build context is the REPOSITORY ROOT, not this directory. bug-347 made
# `repository/` a member of the root cargo workspace and deleted its standalone
# Cargo.lock, so the root lock — the only one CI resolves and tests — is now the
# lock this image must build against. Deploy with:
#
#     fly deploy . --config repository/fly.toml --dockerfile repository/Dockerfile
#
# from the repository root. See DEPLOY.md.

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
COPY repository/Cargo.toml ./repository/
COPY wire/Cargo.toml ./wire/
# `mfb` is a workspace member, so cargo must be able to resolve its manifest —
# but it is never built here. A stub target satisfies the resolver without
# copying the compiler source into the image, which also keeps an edit to
# src/ from invalidating the dependency build below.
#
# `wire` (mfb_wire, plan-126-B) gets no such stub: unlike `mfb` it is a real
# dependency of `mfb_repository`, so it is copied and compiled for real. It is
# `sha2`-only by design — see `wire/src/lib.rs` — precisely so that being in
# this image costs nothing.
RUN mkdir -p src && printf 'fn main() {}\n' > src/main.rs
COPY wire/src ./wire/src
COPY repository/src ./repository/src
# --locked: build the exact versions the root lock pins and CI tests. A lock
# that cannot satisfy the manifest fails the deploy loudly instead of silently
# resolving something else.
RUN cargo build --release --locked --features s3 -p mfb_repository --bin mfb-repo \
    && cp target/release/mfb-repo /usr/local/bin/mfb-repo

FROM debian:bookworm-slim AS runtime

# ca-certificates: TLS trust roots for reaching S3/Tigris over https.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /home/mfb mfb \
    && mkdir -p /data && chown mfb:mfb /data && chmod 700 /data

COPY --from=builder /usr/local/bin/mfb-repo /usr/local/bin/mfb-repo
COPY repository/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

USER mfb
# Persist the metadata SQLite DB (and the server keypair it holds) across
# restarts; on Fly this is a mounted volume.
#
# bug-586: the mode matters, not just the owner. `meta.db` stores the server
# signing private key, the session secret and the online snapshot/timestamp
# keys in plaintext, and SQLite creates the database and its -wal/-shm sidecars
# with the process umask (0644 under the usual 022). `chmod 700 /data` above is
# the image's half; `Store::open_repository` tightens the database and sidecars
# to 0600 at startup and refuses to serve if it cannot. A MOUNTED volume
# replaces this directory's mode with the volume's own, which is why the
# runtime check exists at all — see DEPLOY.md.
VOLUME ["/data"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]

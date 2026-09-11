# bug-586: default container permissions expose the repository private-key database

Last updated: 2026-09-11
Effort: small (1–3h)
Severity: MEDIUM
Class: Security / secret-at-rest permissions

Status: Open
Regression Test: deployment/image test that starts with `umask 022` and asserts
the database directory, SQLite database, WAL, and SHM are inaccessible to other
users.

The supplied Docker deployment creates `/data` with its default mode (`0755`)
and `Store::open_repository` creates `meta.db` with the process umask. Under the
normal `umask 022`, SQLite creates it as `0644`; its WAL/SHM files inherit the
same exposure. The database stores the server signing private key, HMAC session
secret, and online snapshot/timestamp private keys in plaintext. Any distinct
user able to run a process in the container or read the mounted volume can copy
those credentials and forge attestations, sessions, or signed metadata.

Correct behavior: service-owned metadata directories and every SQLite artifact
containing private material are private to the service account (normally `0700`
directory and `0600` files), including existing deployments repaired safely at
startup or rejected with clear operator guidance.

References:

- Security review, 2026-09-11 (deployment and persistence trace)
- `repository/Dockerfile`, `repository/src/store.rs:Store::open_repository`
- `repository/src/store.rs:migrate`, `ensure_server_secret`, and
  `ensure_server_keypair`

## Failing Reproduction

Build the shipped image, invoke the repository initialization path with its
default `/data/meta.db`, and inspect ownership and modes:

```
docker build -f repository/Dockerfile -t mfb-repo .
docker run --rm --entrypoint sh mfb-repo -c 'umask 022; \
  mfb-repo init-root --dbpath /data/meta.db --datapath /tmp/packages \
    --registry-id test; stat -c "%a %U %n" /data /data/meta.db*'
```

- Observed today: `/data` is created by Dockerfile `mkdir` without `chmod`; with
  `umask 022`, `meta.db` is readable by group/other.
- Expected: the data directory and database/WAL/SHM are private to `mfb`, and a
  non-`mfb` account cannot read them.

## Root Cause

`Dockerfile` runs `mkdir -p /data && chown mfb:mfb /data` but never sets a mode.
`Store::open_repository` calls `fs::create_dir_all` and `rusqlite::Connection::open`
without creating the path with restrictive Unix permissions or validating an
existing path. `migrate` then writes raw key/secret columns to the resulting
SQLite files: `server_secrets.secret`, `server_keys.private_key`, and
`registry_config.{snapshot_private,timestamp_private}`.

## Goal

- Make the default Docker/Fly metadata volume private to the service UID.
- Enforce or verify safe permissions for the database and SQLite sidecars.
- Give operators a safe migration path for existing insecure files.

### Non-goals (must NOT change)

- Do not store private signing keys in a public blob backend.
- Do not rely on an unspecified container umask for secret protection.
- Do not silently widen permissions to make an existing deployment start.

## Blast Radius

- `repository/Dockerfile` — default `/data` mode.
- `repository/src/store.rs:Store::open_repository` — creation and existing-file
  checks, including WAL/SHM handling.
- `repository/DEPLOY.md` — operator migration/volume-permission guidance.

## Fix Design

Create the service data directory as mode `0700` in the image. On supported
Unix targets, create or immediately tighten the database and its sidecars to
`0600`, validate existing modes before loading private keys, and document the
operator command needed to repair a mounted volume. Keep this behavior explicit
for non-Unix targets rather than pretending POSIX modes were enforced.

## Phases

### Phase 1 — failing deployment test (no behavior change)

- [ ] Add a `umask 022` regression test for newly created DB/WAL/SHM files.
- [ ] Add coverage for an existing group/world-readable database and directory.

Acceptance: tests show the current default exposes a file holding private keys.
Commit: —

### Phase 2 — the fix

- [ ] Set restrictive image-volume permissions.
- [ ] Enforce secure runtime creation and existing-path validation/repair policy.
- [ ] Document upgrade steps for mounted volumes.

Acceptance: no other UID can read service key material under default deployment.
Commit: —

### Phase 3 — full validation

- [ ] Run `cargo test -p mfb_repository`.
- [ ] Build and inspect the Docker image with default and mounted-volume paths.

Acceptance: all SQLite artifacts carrying secrets remain service-private.
Commit: —

# bug-586: default container permissions expose the repository private-key database

Last updated: 2026-09-12
Effort: small (1–3h)
Severity: MEDIUM
Class: Security / secret-at-rest permissions

Status: **FIXED** (`9f1a0879d`).
Regression Test: `store::tests::open_repository_makes_the_key_bearing_database_private`
and `store::tests::open_repository_leaves_an_already_private_database_alone_and_still_works`
in `repository/src/store.rs`.

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

- [x] Add a regression test for newly created DB/WAL/SHM files.
- [x] Add coverage for an existing group/world-readable database and directory.

**Measured**, independently of this repo, under `umask 022`:

    data                   755
    data/meta.db           644
    data/meta.db-wal       644
    data/meta.db-shm       644

The report was correct, including that the sidecars inherit the exposure —
protecting only `meta.db` would leave the same secrets readable in the WAL.

**One deliberate departure from the filed test plan.** The doc asked for a test
that "starts with `umask 022`". `umask` is process-global and libtest runs its
tests in parallel, so a test that set it would race every other test in the
binary that creates a file — the same class of hazard as a global panic hook.
The RED is made deterministic instead by pre-creating the directory at `0755`
and an empty (hence valid, empty) SQLite database at `0644`. That proves the
same defect without the race **and** covers strictly more: it is also the
already-deployed-and-exposed volume, i.e. the upgrade path the Goal asks for.
Commit: — (tests landed with the fix, `9f1a0879d`)

### Phase 2 — the fix

- [x] Set restrictive image-volume permissions.
- [x] Enforce secure runtime creation and existing-path validation/repair policy.
- [x] Document upgrade steps for mounted volumes.

Two halves, because **neither is sufficient alone** — and this is the part the
report did not spell out:

- `Dockerfile` creates `/data` as `0700`. A **mounted volume replaces that
  directory's mode with its own**, so on Fly the image half does not survive
  `fly volumes create`. That is precisely why a runtime check is required and
  not merely belt-and-braces.
- `Store::open_repository` tightens the directory to `0700` *before* the
  database is created — closing the window in which SQLite has just created it
  at `0644` — then tightens the database and both sidecars to `0600` after WAL
  is enabled and **before** `migrate` / `ensure_server_secret` /
  `ensure_server_keypair` write any key material.

`make_private` only ever REMOVES access: a path with no group/other bits is left
alone, so a deliberate `0400` survives and the fix can never widen an operator's
hardening. When a path is exposed and cannot be tightened (a volume owned by
another UID) the open FAILS, naming the path and the repair command, rather than
serving with the keys readable — "start anyway" is exactly the silent widening
the non-goals forbid.

Non-Unix is an explicit no-op with a comment saying so, per the Fix Design:
claiming enforcement that did not happen is worse than stating it did not.

Acceptance: **met.**
Commit: `9f1a0879d`

### Phase 3 — full validation

- [x] Run `cargo test -p mfb_repository`.
- [ ] Build and inspect the Docker image — **not done, and named as a gap.**

`cargo test -p mfb_repository --no-fail-fast` -> **347 + 21 passed, 0 failed,
exit 0**, in an isolated `git worktree add --detach`.
`cargo fmt -p mfb_repository -- --check` -> exit 0.

**Name the instrument, including where it does not reach.** The artifact gate is
structurally blind to this crate (no IR, no goldens), so its silence proves
nothing; the `mfb_repository` unit suite is the instrument for the runtime half
and it covers it directly. The **image half is covered by neither**: no test in
this repo builds `repository/Dockerfile`, so `chmod 700 /data` is verified by
reading it, not by running it. That is stated rather than papered over. It is
also the half that matters least in practice, because a mounted volume overrides
it — which is why the runtime check is the load-bearing one and is tested.

Acceptance: all SQLite artifacts carrying secrets remain service-private —
**met for the runtime path, which is the one that survives a volume mount.**
Commit: `9f1a0879d`

## Outcome

Fixed in `9f1a0879d`.

The transferable part: **a permissions fix in an image is undone by the volume
it is supposed to protect.** `chmod 700 /data` in a `Dockerfile` is correct and
almost irrelevant — the mount replaces that inode. Any at-rest permission
guarantee for data on a mounted volume has to be re-established by the process
at startup, every start, or it is a guarantee about the empty directory the
image shipped.

Second: **tightening and widening are not symmetric, and the test has to say
which one it forbids.** The obvious implementation — `set_permissions(0o600)`
unconditionally — passes every negative test and silently relaxes an operator's
`0400`. Asserting the `0400` survives is what makes the fix "remove access
only" rather than "set access to what I assumed".

# Deploying `mfb-repo` to Fly.io

The package-registry server runs as a single Fly machine: package **metadata**
lives in a SQLite database on a Fly **volume**, and package **blobs** live in
S3-compatible object storage (Fly's built-in [Tigris] works out of the box).
Blob downloads are served as a redirect to a short-lived presigned URL, so the
app never proxies blob bytes.

Files in this directory:

- `Dockerfile` — builds `mfb-repo` with the `s3` feature (multi-stage, non-root).
- `docker-entrypoint.sh` — maps env vars → `mfb-repo` CLI args.
- `fly.toml` — app config: one machine, a `/data` volume, `/health` checks.

## The build context is the repository root

`fly deploy` **must be run from the repository root**, not from here:

```sh
fly deploy . --config repository/fly.toml --dockerfile repository/Dockerfile
```

`repository/` used to be its own cargo workspace with its own `Cargo.lock`, so
the image could be built from this directory alone. bug-347 made it a member of
the root workspace and deleted that lock — the root `Cargo.lock` is now the only
one cargo resolves and CI tests, and it lives one level up. A bare `fly deploy`
from here fails with `"/Cargo.lock": not found`.

The Dockerfile builds `-p mfb_repository --locked`, so the deployed binary pins
exactly the dependency versions the test suite ran against. The root
`.dockerignore` keeps the context to `Cargo.toml`, `Cargo.lock`, and
`repository/`; `repository/.dockerignore` is inert now that the context root
moved (docker reads only the one at the context root).

## One-time setup

Run the `fly apps`/`volumes`/`storage`/`scale` commands below **from this
directory** (`repository/`) — `flyctl` reads the app name from the `fly.toml` in
the working directory, and from anywhere else they either target the wrong app
or fail with `app not found`. Pass `-a <app>` explicitly if you must run them
from elsewhere. `fly deploy` is the exception: it runs from the root, as above.

```sh
# 1. Pick a unique app name. Set `app = "<name>"` in fly.toml FIRST, then create
#    the app under that exact name — every later command resolves the app
#    through fly.toml, so a name that disagrees with it fails at step 2.
#    fly.toml ships with `app = "mfb-repo"`; keep it only if that name is free.
fly apps create <name>                 # or: fly launch --no-deploy --copy-config

# 2. Persistent volume for the metadata DB + server keypair.
#    Create it in the same region as `primary_region`.
fly volumes create mfb_repo_data --region iad --size 1

# 3. Blob storage — Fly object storage (Tigris). This provisions a bucket and
#    sets BUCKET_NAME, AWS_ENDPOINT_URL_S3, and AWS_* credential secrets on the
#    app; the entrypoint derives `--datapath s3://<bucket>/packages` from them.
fly storage create

#    --- OR use an external S3 bucket instead of Tigris: ---
# fly secrets set MFB_REPO_DATAPATH=s3://my-bucket/packages \
#     AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... AWS_REGION=us-east-1
#    (add MFB_REPO_S3_ENDPOINT=https://... for a non-AWS S3-compatible store)

# 4. Deploy — from the REPOSITORY ROOT (see "The build context" above).
cd .. && fly deploy . --config repository/fly.toml \
    --dockerfile repository/Dockerfile && cd repository

# 5. Keep it to a single machine (SQLite + volume are single-writer).
fly scale count 1
```

## Initialize the root of trust

The signed-metadata root ceremony runs once, against the deployed volume DB:

Substitute your real bucket name for `<bucket>` (`fly storage list`, or
`fly secrets list` to confirm `BUCKET_NAME` is set). Do **not** write
`$BUCKET_NAME` here: `-C` takes a single string that your *local* shell expands
first, so the variable resolves to the empty string on your machine and the
server sees `s3:///packages`, which is rejected as naming no bucket.

```sh
fly ssh console -u mfb -C "mfb-repo init-root --dbpath /data/meta.db \
    --datapath s3://<bucket>/packages --registry-id my-registry"
```

`init-root` touches only the metadata DB — it recognizes an `s3://` data path
and skips the blob backend entirely, so it needs no `--s3-endpoint` even on
Tigris.

`-u mfb` matters: `fly ssh console` connects as **root** by default, but the
image runs the server as the unprivileged `mfb` user (uid 10001) that owns
`/data`. Writing the DB as root leaves root-owned SQLite files the server then
cannot write.

### Volume permissions (bug-586)

`/data/meta.db` holds the **server signing private key**, the session-signing
secret, and the online snapshot/timestamp private keys — all in plaintext. Its
`-wal` and `-shm` sidecars carry the same rows. SQLite creates all three with
the process umask, so under the ordinary `umask 022` they would land at `0644`.

Two halves protect them, because neither is sufficient alone:

- The image creates `/data` as `0700` (`Dockerfile`). A **mounted volume
  replaces that directory with its own mode**, so on Fly this does not survive
  a `fly volumes create`.
- `mfb-repo` therefore tightens the directory to `0700` and the database and
  both sidecars to `0600` every time it opens the store, before it writes any
  key material. It only ever removes access — an operator who has deliberately
  narrowed further (say `0400`) keeps that.

If the files are exposed and cannot be tightened — normally a volume owned by
another UID — startup **fails** with a message naming the path, rather than
serving with the keys readable. Repair it and restart:

```sh
fly ssh console -C "chown -R mfb:mfb /data && chmod 700 /data && chmod 600 /data/meta.db*"
```

(as root, i.e. without `-u mfb`). Existing deployments are repaired
automatically on the next restart when the service account already owns the
files; the command above is only needed when it does not.

Store the printed **root PRIVATE key** offline — it is never persisted on the
server. Pin the printed root fingerprint out of band. (`reanchor` is likewise
run via `fly ssh console`.)

### Renewing it (bug-584)

`init-root` really is once: run against a registry that already has a root of
trust it **refuses**, because overwriting the anchor is indistinguishable, from
a pinned client's side, from a takeover. To extend the root expiry and rotate
the online snapshot/timestamp keys, use `renew-root` with the offline root key
you stored above. Write the key to a file — never pass it as an argument, which
would put it in the process table — and delete the file afterwards:

```sh
fly ssh console -u mfb -C "sh -c 'umask 077; cat > /data/root.key'" < root.key
fly ssh console -u mfb -C "mfb-repo renew-root --dbpath /data/meta.db \
    --datapath s3://<bucket>/packages --registry-id my-registry \
    --root-key-file /data/root.key --expires-days 365"
fly ssh console -u mfb -C "rm -f /data/root.key"
```

The root fingerprint does **not** change, so every already-pinned client keeps
verifying with no out-of-band step; the command prints the unchanged fingerprint
so you can confirm that. It is safe while the server is serving.

If the offline root key is **lost**, the only recovery is `mfb-repo
reanchor-root` (same flags as `init-root`), which mints a new anchor and prints
a new fingerprint. Every client that pinned the old one fails hard until it
re-pins out of band, so treat it as a published security event, not routine
maintenance.

## Reclaiming abandoned uploads

`PUT /blob` accepts a vendored library **before** any package version references
it, so a publish abandoned between the upload and the commit leaves bytes nothing
will ever name. `mfb-repo gc` reclaims them:

Unlike `init-root`, `gc` really reaches the blob store, so on Tigris (or any
S3-compatible store) it needs `--s3-endpoint`. The server picks that endpoint up
from `AWS_ENDPOINT_URL_S3` via `docker-entrypoint.sh`, but `fly ssh console -C`
execs the binary directly and bypasses the entrypoint — omit the flag and the
AWS SDK silently targets real AWS S3 instead of your bucket. Read the value your
app actually has with `fly ssh console -C "printenv AWS_ENDPOINT_URL_S3"`
(Tigris is `https://fly.storage.tigris.dev`), and substitute `<bucket>` as above.

```sh
# Dry run — lists every unreachable blob with its size, age, and location.
fly ssh console -u mfb -C "mfb-repo gc --dbpath /data/meta.db \
    --datapath s3://<bucket>/packages \
    --s3-endpoint https://fly.storage.tigris.dev"

# Same thing, then actually delete.
fly ssh console -u mfb -C "mfb-repo gc --dbpath /data/meta.db \
    --datapath s3://<bucket>/packages \
    --s3-endpoint https://fly.storage.tigris.dev --delete"
```

It is a dry run unless `--delete` is given, and it never runs on its own — the
background reaper expires auth ephemera only and never touches package content.

- A blob **any live version references is never deleted**, including a *yanked*
  one: yanking means "do not resolve this by default", not "delete it", and
  lockfiles pinning the hash must keep installing.
- A blob **younger than the grace period** (default 24h, `--grace-hours N`) is
  never deleted either. There is no lock between the upload and the publish, so
  a publisher's in-flight blobs are legitimately unreachable until the publish
  lands; the window is what makes the sweep safe. `--grace-hours 0` is refused.
- `--json` emits a machine-readable report including the reachable-side total,
  for scripting. Any per-blob failure exits nonzero.

Safe to run as often as you like. A registry that never runs it behaves exactly
as it always has.

## Verify

```sh
curl https://<app>.fly.dev/health      # {"ok":true}
curl https://<app>.fly.dev/ident       # the server's stable public key
```

Point a client at the registry with `MFB_REPO_URL=https://<app>.fly.dev`. The
client follows the presigned-URL redirect on `GET /blob/<hash>` and re-hashes
the downloaded bytes, so integrity holds end-to-end.

## Notes

- **Single machine only.** The volume binds to one machine and SQLite is
  single-writer; do not `fly scale count` above 1. For higher availability you
  would need to move metadata off SQLite (out of scope here).
- **Back up the volume.** It holds the server keypair — losing it changes the
  server identity (`/ident`), which clients pin. Use `fly volumes snapshots`.
- **Costs.** `auto_stop_machines` stops the machine when idle and restarts it on
  the next request; set `min_machines_running = 1` in `fly.toml` for always-on.

[Tigris]: https://fly.io/docs/reference/tigris/

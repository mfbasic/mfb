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

## One-time setup

```sh
# 1. Pick a unique app name (edits fly.toml).
fly apps create my-mfb-repo            # or: fly launch --no-deploy --copy-config
#    then set `app = "my-mfb-repo"` in fly.toml

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

# 4. Deploy.
fly deploy

# 5. Keep it to a single machine (SQLite + volume are single-writer).
fly scale count 1
```

## Initialize the root of trust

The signed-metadata root ceremony runs once, against the deployed volume DB:

```sh
fly ssh console -C "mfb-repo init-root --dbpath /data/meta.db \
    --datapath s3://$BUCKET_NAME/packages --registry-id my-registry"
```

Store the printed **root PRIVATE key** offline — it is never persisted on the
server. Pin the printed root fingerprint out of band. (`reanchor` is likewise
run via `fly ssh console`.)

## Reclaiming abandoned uploads

`PUT /blob` accepts a vendored library **before** any package version references
it, so a publish abandoned between the upload and the commit leaves bytes nothing
will ever name. `mfb-repo gc` reclaims them:

```sh
# Dry run — lists every unreachable blob with its size, age, and location.
fly ssh console -C "mfb-repo gc --dbpath /data/meta.db \
    --datapath s3://$BUCKET_NAME/packages"

# Same thing, then actually delete.
fly ssh console -C "mfb-repo gc --dbpath /data/meta.db \
    --datapath s3://$BUCKET_NAME/packages --delete"
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

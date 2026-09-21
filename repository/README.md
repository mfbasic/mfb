# `mfb_repository`: the MFBASIC package registry

This crate is the **package registry** for MFBASIC. It contains both halves of
the registry protocol:

- **`mfb-repo`**, the reference registry server. It is a single self-contained
  binary built on axum, SQLite and a pluggable blob store. The public instance
  runs at `https://mfb-repo.fly.dev`.
- **The registry client library** that the `mfb` compiler links against. It
  backs `mfb repo …`, `mfb pkg add/install/verify`, `mfb build --sign`,
  `mfb key rotate`, `mfb machine revoke` and `mfb info`.

The registry stores and serves signed `.mfp` packages. Its main job is to make
publisher identity verifiable. A package can only be published by someone who
holds the account's **ident private key** *and* has a **live authenticated
session**. Every state change goes into a public, append-only **transparency
log**, so misbehavior leaves evidence, even misbehavior by the registry itself.

The normative contract is the `package-manager` spec (see
[Specifications](#specifications)). This README is an orientation guide. Where
the two disagree, the spec wins.

## Where it fits

```text
 ┌──────────── mfb (compiler / CLI) ────────────┐        ┌──────── mfb-repo ────────┐
 │ mfb repo register / auth / link / publish    │  HTTP  │ axum router (server.rs)  │
 │ mfb build --sign   mfb pkg add / verify      │ ─────▶ │ SQLite metadata (store)  │
 │        uses mfb_repository::client           │  JSON  │ blob store: dir or S3    │
 │ keys + sessions in ~/.mfb (local.rs)         │        │ transparency log (log)   │
 └──────────────────────────────────────────────┘        └──────────────────────────┘
                       both depend on mfb_wire (byte primitives, .mfp framing,
                       name/version validators, terminal sanitizer)
```

The crate is a member of the root cargo workspace (bug-347). `mfb` depends on
it, and it depends on `mfb_wire` (`../wire`), which is the one crate both of
them are allowed to share. `mfb_repository` must never depend on `mfb`.

## The trust model in one page

There are four kinds of Ed25519 keypair. Where each private key is stored
determines what that key is allowed to do.

| Key         | Count               | Private half lives               | Job                                        |
| ----------- | ------------------- | -------------------------------- | ------------------------------------------ |
| **server**  | 1 per registry      | the server (its only private key) | signs attestations, name bindings, checkpoints |
| **ident**   | 1 per account       | every linked machine             | *is* the identity; signs build proofs      |
| **auth**    | 1 per machine       | that machine                     | logs into the API, nothing more            |
| **signing** | 1 per build, one-off | memory, for one build only      | signs the `.mfp` bytes, then is discarded  |

A signed build works like this:

1. `mfb build --sign <owner>` generates a one-off signing keypair.
2. The client calls `POST /signing` with a session token. The server records the
   request and then returns an **attestation**: a server-signed JSON statement
   that names the exact `ident@version`, the ident fingerprint and the one-off
   key's fingerprint.
3. The client mints a **proof**: the same facts, signed with the ident key.
4. The one-off key signs the package, and the proof and attestation are embedded
   in the `.mfp` header.

`/validate` and `/publish` check the whole chain again on the server. The checks
are: wire integrity, the session owner, the attestation signature, the
attestation pinning this exact package, the signing key matching, the ident
matching the *current* name binding, the proof, and the payload hash and package
signature. Because of this:

- a stolen **auth** session can request attestations, which are logged, but it
  can never produce a package that verifies;
- a stolen **ident** key without a session cannot get an attestation;
- a fully compromised **server** holds no user private keys.

Every signing domain is a NUL-terminated, versioned string such as
`mfb-repo-auth-v1\0` or `MFP-PROOF-v1\0`. Including the domain in the signed
bytes means a signature made for one purpose cannot be replayed for another. The
full table is in the signing spec.

### Other defenses built on that model

- **Transparency log** (`log.rs`, `/log/*`). The log is an RFC 6962 Merkle log.
  Each register, attestation, publish, link, revoke, rotate, re-anchor, org-role,
  token, transfer, release-state and root change appends one entry *in the same
  transaction* as the change. Clients pin the last checkpoint they saw and fail
  hard on a rollback or a fork. `repo publish` checks that its own publish entry
  is included in the log.
- **Server key pinning.** `GET /ident` is pinned as `server.pub` the first time a
  client connects. `/index` responses carry a server-signed
  **name binding** (`owner → ident fingerprint`), which is what a first
  `pkg add` pins.
- **Ident rotation and re-anchor.** `/keys/rotate` links the new ident to the old
  one with a signature from the old ident. Consumers can follow that chain
  (`GET /idents/<owner>`) without trusting the server. If an ident changes with
  *no* chain link, clients fail hard. Only the operator can do that, through
  `mfb-repo reanchor`, and there is deliberately no HTTP route for it.
- **Machine linking.** The ident key is copied to a new machine. It is sealed
  with ChaCha20-Poly1305 under an argon2id key derived from a one-time pairing
  code (about 125 bits) that the server never sees. The encrypted blob can be
  fetched once, expires after 600 s, and is released only against a signature
  from a separate approval key that is also derived from the pairing code.
- **TUF-style signed metadata** (`/root.json`, `/snapshot.json`,
  `/timestamp.json`). An offline root key delegates the server, snapshot and
  timestamp keys. This lets clients detect stale or partial indexes served by a
  mirror or a man-in-the-middle. It is opt-in per client through
  `mfb repo trust`.
- **Accounts.** Orgs, scoped short-lived publish tokens for CI, and two-sided
  ownership transfers. Each account change needs a live session *and* an ident
  signature, and each one is logged.

## How the server works

| Concern | Where | Notes |
| ------- | ----- | ----- |
| HTTP surface | `src/server.rs` (`build_router`) | JSON on the wire uses camelCase. Binary data is base64url with no padding. Hashes and fingerprints are lowercase hex. |
| Persistence | `src/store.rs` | SQLite in WAL mode with a busy timeout. Holds owners, keys, sessions, challenges, the log, packages and versions, blob refs, docs, orgs, tokens, transfers, and registry config. The schema is created and migrated in place when the store opens. |
| Blobs | `src/blobstore.rs` | Content-addressed. `<hash>.mfp` holds a package and `<hash>.bin` holds a vendored native library. The backend is a local directory or `s3://bucket/prefix`. |
| `.mfp` parsing | `src/package.rs`, `src/abi.rs` | Reads the container v1.0 header and the MFPC sections the registry needs: the ABI index, the section-10 native-library table, and the manifest. Uses `mfb_wire::mfp` framing. |
| Transparency log | `src/log.rs` | RFC 6962 tree head, inclusion proofs and consistency proofs. |
| Crypto | `src/crypto.rs` | Ed25519, fingerprints, signing-domain messages, and pairing-blob sealing. |
| Web UI | `src/web/` | Anonymous, read-only HTML rendered on the server with `maud`, which escapes values by default. |
| Operator tools | `src/gc.rs`, `src/backfill.rs`, `src/main.rs` | Subcommands for blob GC, metadata backfill, and the root and re-anchor ceremonies. |
| Client | `src/client.rs`, `src/local.rs` | Blocking `reqwest` over rustls, plus the `~/.mfb` key and session store. |

**Endpoint groups.** The protocol spec has the full table.

- *Liveness and identity:* `GET /health`, `GET /ident`.
- *Accounts and auth:* `/accounts/register`, `/auth/challenge` → `/auth/login`
  (issues an HS256 JWT that lasts 3600 s), `/machines/link[/fetch]`,
  `/machines/revoke[/challenge]`, `/keys/rotate`, `GET /idents/<owner>`.
- *Signing and publishing:* `/signing`, `/validate`, `/publish`, `/release-state`,
  `/orgs/members`, `/tokens[/revoke]`, `/packages/transfer/{offer,accept}`.
- *Install:* `GET /index/<owner>%23<package>` and
  `GET|HEAD|PUT /blob/<hash>`.
- *Log and metadata:* `/log/checkpoint`, `/log/proof/<i>`, `/log/consistency`,
  `/log/publish`, `/root.json`, `/snapshot.json`, `/timestamp.json`.
- *Anonymous read and web:* `/search`, `/packages/<ident>[/audit|/docs]`, `/`,
  `/search.html`, `/p/<ident>[/audit|/docs]`, `/style.css`.

**Invariants that are there on purpose.** Don't "simplify" these away:

- **No cookies, anywhere.** Session tokens travel in JSON request bodies. The one
  exception is `PUT /blob`, which uses a bearer header. As a result, the server
  has no CSRF surface. Adding cookie auth to anything would make every mutating
  POST vulnerable to CSRF.
- **The read surface never takes a credential and always lists every release
  state**, including yanked and deprecated versions. That lets third parties
  detect a registry that hides versions. `/audit` returns inclusion proofs, not
  bare log indexes.
- **HTML pages ship no JavaScript and set a strict CSP**
  (`default-src 'none'`, with no `script-src`). Links rendered from
  publisher-controlled URLs go through an http/https allowlist.
- **Blob writes follow stage → DB row → promote**, so a blob that can be served
  always has a committed row. `PUT /blob` checks `sha256(body)` before storing
  anything. `/publish` refuses to proceed if a vendored-library hash in the
  package has no blob.
- **Package content is never deleted automatically.** The background reaper runs
  every 60 s and only expires challenges, sessions and pairing blobs. Blob GC
  runs only when an operator starts it.
- **Hardening:** a 64 MiB request-body cap, sliding-window rate limits on the
  auth, signing, publish and search routes, `?limit` clamped to 50 on `/search`,
  and warn-only typosquat warnings at publish.

## Running it

### Local development server

```sh
cargo run -p mfb_repository --bin mfb-repo -- \
    --dbpath /tmp/mfb-repo/meta.db --datapath /tmp/mfb-repo/blobs
# prints MFB_REPO_LISTEN=127.0.0.1:7777 once bound
```

Then point the `mfb` client at it:

```sh
export MFB_REPO_URL=http://127.0.0.1:7777   # loopback http is exempt from the TLS requirement
export MFB_HOME=/tmp/mfb-home               # optional: keep test keys out of ~/.mfb
mfb repo register alice
mfb repo auth alice
mfb repo publish alice path/to/package
mfb pkg add 'alice#mypkg'
```

On first start the server generates its own keypair and session secret and
stores them in the metadata database. Neither leaves that database. At startup
the server tightens the database and its `-wal`/`-shm` sidecar files to `0600`
and their directory to `0700`. If it cannot do that, it refuses to serve
(bug-586).

### `mfb-repo` command line

```text
mfb-repo --dbpath <db> --datapath <dir|s3://bucket/prefix> [--listen addr:port] [--s3-endpoint url]
mfb-repo init-root         --dbpath … --datapath … --registry-id <id> [--expires-days n]
mfb-repo renew-root        --dbpath … --datapath … --registry-id <id> --root-key-file <path> [--expires-days n]
mfb-repo reanchor-root     --dbpath … --datapath … --registry-id <id> [--expires-days n]
mfb-repo reanchor          --dbpath … --datapath … --owner <owner> --ident-key <base64url>
mfb-repo gc                --dbpath … --datapath … [--s3-endpoint url] [--grace-hours n] [--delete] [--json]
mfb-repo backfill-metadata --dbpath … --datapath … [--s3-endpoint url]
```

- **`init-root`** can run only once. It creates the offline root key, prints it
  once, and never stores it. **`renew-root`** keeps the same anchor, so pinned
  clients carry on without doing anything. **`reanchor-root`** is only for
  recovering from a *lost* root key, and it breaks every client that pinned the
  old root.
- **`gc`** is a dry run unless you pass `--delete`. It never removes a blob that
  a live version references (yanked versions count as live). It also never
  removes a blob younger than the grace period, which defaults to 24 h;
  `--grace-hours 0` is refused.
- **`backfill-metadata`** re-parses stored blobs to fill in author, URL, target,
  and documentation records for older versions. It is idempotent, skips any blob
  it cannot parse, and exits nonzero if it skipped anything.

### S3 and production

S3 support is behind a cargo feature, so the AWS SDK is not compiled into
`mfb`:

```sh
cargo build -p mfb_repository --features s3 --bin mfb-repo
```

In S3 mode, `GET /blob` returns a `302` redirect to a short-lived presigned URL,
so blob bytes never pass through the server. The client re-hashes every download
in either mode. The metadata database always stays on local disk.

Production runs on Fly.io as **exactly one machine**: SQLite on a volume, and
blobs in Tigris or any S3-compatible store. The files are `Dockerfile`,
`docker-entrypoint.sh` (maps environment variables to CLI flags) and `fly.toml`.
The build context is the **repository root**, not this directory. See
[`DEPLOY.md`](DEPLOY.md) for the full walkthrough, including the root-of-trust
ceremonies and running `gc` against Tigris.

## Testing

```sh
cargo test -p mfb_repository          # unit + router tests (a bare `cargo test` at the root runs them too)
```

Most tests are inline `#[cfg(test)]` modules. Router tests drive
`build_router` through `tower::ServiceExt::oneshot` without binding a socket.
`tests/s3_backend.rs` is a live test against MinIO. It is ignored by default and
needs `--features s3`; the comment at the top of that file has the full
command. End-to-end client/server behavior is also covered by the compiler's
test suites in the root crate.

## Specifications

The contract lives in the compiler's spec tree. Read it with
`mfb spec package-manager <topic>`, or open the files under
`src/docs/spec/package-manager/`:

- [`01_repository-protocol.md`](../src/docs/spec/package-manager/01_repository-protocol.md):
  endpoints, request and response shapes, auth, the log, install, blobs, GC,
  release states, accounts, signed metadata, and validate-then-publish.
- [`02_key-store.md`](../src/docs/spec/package-manager/02_key-store.md): the
  `~/.mfb/<repo-hash>/` layout and its permissions.
- [`03_signing.md`](../src/docs/spec/package-manager/03_signing.md): the four
  keys, signing domains, proof and attestation, and executable signing.
- [`04_owner-names.md`](../src/docs/spec/package-manager/04_owner-names.md): the
  owner-name grammar (`[A-Za-z_][A-Za-z0-9_]*`, at most 255 bytes, `std`
  reserved).

The `.mfp` byte format that the registry parses is specified separately under
`src/docs/spec/package/`.

## License

MIT. See [`../LICENSE`](../LICENSE).

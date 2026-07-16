# plan-48-A: repository native-library blobs (server + protocol)

Last updated: 2026-07-16
Overall Effort: large (4h–1d) — the whole plan-48 feature (A + B)
Effort: medium (2h–4h)
Depends on: plan-46-B (section 10 must exist to reference blobs by hash)

Teaches the `mfb-repo` registry to store a package's **vendored native library
files as separate content-addressed blobs**, alongside — never inside — its
`.mfp`. Adds the missing write half of the blob API (`PUT`/`HEAD /blob/<hash>`),
generalizes the blob store beyond `.mfp` artifacts, and makes `POST /publish`
refuse a package whose section-10 table references a blob the registry does not
have.

This is the deferred distribution half that plan-46-C §4.4 and plan-46-D
explicitly scoped out. plan-48-B is the client side.

The single behavioral outcome: a binding whose `.mfp` carries a section-10
`vendor` locator can have that locator's file uploaded as its own blob, fetched
back byte-identically through the existing `GET /blob/<hash>`, and the registry
rejects a `publish` that would leave a section-10 hash dangling.

References (read first):

- `repository/src/server.rs` — router (`:636-684`), `MAX_BODY_BYTES` (`:592`),
  `DefaultBodyLimit::max` layer (`:666`), `package_blob` GET handler (`:1312`,
  hex validation `:1318-1326`), `publish_package` (`:1762`), the
  stage→commit→promote block (`:1806-1841`), `PackageArtifactRequest` (`:437-449`),
  `verify_session_token` (`:2032-2046`), and the ABI-index trust comment
  (`:2010-2013`) whose argument this plan reuses.
- `repository/src/blobstore.rs` — `BlobFetch` (`:29`), `StagedBlob` (`:40`),
  `BlobStore` enum (`:143`), `blob_name` (`:136`), `exists` (`:172`),
  `stage`/`promote`/`abort` (`:183`/`:200`/`:219`), `get` (`:234`), S3 impl
  (`:251+`, `PRESIGN_TTL` `:261`).
- `repository/src/store.rs` — `package_blobs` schema (`:235-239`),
  `package_versions` schema (`:224-233`), the `INSERT OR IGNORE` (`:1112`),
  `reap_expired` (`:1754-1771` — note it does **not** touch package blobs).
- `repository/src/package.rs` — `parse_mfp_package` (`:152`),
  `verify_package_signature` (`:231-239`), `verify_payload_hash` (`:243-248`).
- `src/docs/spec/package-manager/01_repository-protocol.md` — the endpoint table
  and encoding rules this plan extends.
- `planning/plan-46-B-native-library-table-section.md` §4.1 — the section-10 wire
  format whose 32-byte `hash` field is the blob key.

## 1. Goal

- `PUT /blob/<hash>` accepts raw native-library bytes from an authenticated
  publisher, verifies `sha256(body) == <hash>` before storing, and is idempotent
  (re-uploading an existing blob is a cheap success, not an error).
- `HEAD /blob/<hash>` answers "do you already have this?" so a publisher skips
  re-uploading an unchanged 40 MB library on every version bump.
- `GET /blob/<hash>` serves a native blob exactly as it serves a `.mfp` today —
  inline bytes with the corruption re-check, or a `302` to a presigned URL in S3
  mode. **No change to the download path's shape.**
- `POST /publish` parses the `.mfp`'s section 10 and **refuses** the publish if
  any `vendor` locator's hash is absent from the blob store, so a published
  package can never reference a blob that does not exist.
- The registry records which blobs a package version references, so a future GC
  has the reachability data it needs.

### Non-goals (explicit constraints)

- **No client changes** — `pkg publish` uploading blobs and `pkg add`
  downloading them is plan-48-B.
- **No streaming upload.** Blob bodies are buffered like every other request on
  this server. The existing `MAX_BODY_BYTES` = 64 MiB (`server.rs:592`) becomes
  the native-blob ceiling; raw bytes already lift the effective limit from
  ~48 MiB (base64) to a full 64 MiB. Streaming and a higher cap are real future
  work (§4.6), not this plan.
- **No presigned PUT.** Upload goes through the server so it can verify the hash
  before storing. S3-mode *download* keeps its presigned 302; only the write path
  is proxied. See §4.6.
- **No garbage collection.** The registry has none today (§2.4) and this plan does
  not add it — it only records the reachability rows a future GC would need.
- No change to the signing/trust chain, the proof/attestation payloads, the
  transparency log's entry shape, or `mfb.lock`. §3.2 explains why none is needed.
- No multipart. §3.1 records why.

## 2. Current State

### 2.1 The server and its blob API

`mfb-repo` is an **axum 0.7** service (`repository/` crate, `Cargo.toml:22`),
router at `server.rs:636-684`. Its only blob surface is **`GET /blob/:hash`**
(`server.rs:1312`), which hard-validates 64 lowercase hex chars
(`server.rs:1318-1326`) and answers via `BlobFetch` (`blobstore.rs:29`):
`Bytes` → `200 application/octet-stream` with `Cache-Control: immutable` and a
re-hash corruption check (`server.rs:1339-1346`), or `Redirect` → `302` +
presigned `Location`, `Cache-Control: no-store` (`server.rs:1347-1352`).

**There is no PUT or POST blob endpoint.** Blobs are written *only* as a side
effect of `POST /publish` (`server.rs:1762`), which base64-decodes the single
`artifact` field of `PackageArtifactRequest` (`server.rs:437-449`), checks
`blob_store.exists` for dedup (`server.rs:1801`), then runs
**stage → DB commit → promote**, with `abort` on DB failure
(`server.rs:1806-1841`).

Storage is an **enum, not a trait** (`blobstore.rs:143`): `Local` (write
`<hash>.mfp.tmp-<uuid>`, atomic rename on promote) or `S3` (`put_object` straight
to the final content-addressed key; `promote` is a no-op, `abort` deletes),
behind the `s3` cargo feature (`Cargo.toml:17`, off by default so the AWS SDK
stays out of core `mfb` builds).

### 2.2 Two hard blockers in the existing design

**`blob_name` hardcodes the `.mfp` suffix** (`blobstore.rs:136`):

```rust
fn blob_name(hash: &str) -> String { format!("{hash}.mfp") }
```

Every backend method — `blob_ref`, `exists`, `stage`, `get`, and the S3 `key()`
(`blobstore.rs:300-302`) — funnels through it. A native library stored today
would land as `<sha256-of-a-.so>.mfp`, which is a lie in the filename and in the
S3 keyspace.

**Nothing relates a version to more than one blob.** `package_versions` carries a
scalar `hash TEXT NOT NULL` (`store.rs:224-233`), and `package_blobs` is a flat
global map with no kind and no back-reference:

```sql
CREATE TABLE package_blobs (hash TEXT PRIMARY KEY, path TEXT NOT NULL, created_at INTEGER NOT NULL)  -- store.rs:235-239
```

`path` is documented as informational only (`blobstore.rs:157-158`); serving is
by hash. The insert is `INSERT OR IGNORE` (`store.rs:1112`), so two versions
sharing a hash collapse to one row with nothing counting the references.

### 2.3 Multipart does not exist here

Verified: **zero** occurrences of `multipart` / `boundary` / `form-data` /
`Content-Disposition` in `repository/` Rust source. `multer` — axum 0.7's
multipart backend — has **zero** occurrences in `repository/Cargo.lock`;
`multipart` is not a default axum feature and `axum-extra` is absent. Adding
multipart means adding a **dependency**, not wiring a feature.

(Search hazard: the MFB *language's* stdlib `http` package does spec multipart
parsing — `src/docs/spec/stdlib/05_http.md:248`, `src/docs/man/builtins/http/package.md`.
That is a `.mfb`-implemented runtime facility for compiled programs and shares no
code with this server. Do not let it confuse a grep.)

### 2.4 No garbage collection

`package_blobs` has no refcount and no reverse index. `reap_expired`
(`store.rs:1754-1771`) deletes only `auth_challenges`, `sessions`, and
`pairing_blobs` — the latter are **auth ephemera, not package blobs**, and the
comment at `server.rs:624-625` is easy to misread. The only blob deletion
anywhere is `abort()` on a failed publish (`blobstore.rs:219-231`). Blobs are
permanent by design, bounded admissionally by `MAX_VERSIONS_PER_OWNER = 10_000`
(`server.rs:611`) and `PUBLISH_PER_OWNER_MAX = 30`/60s (`server.rs:607`).

That economics was fine for `.mfp` files of IR. Native libraries are orders of
magnitude larger, ×7 target slots (plan-46-B §4.2). Permanent-blob storage growth
is a real operational question this plan **surfaces but does not solve** (§Open
Decisions).

### 2.5 Diagnostic codes

Registry HTTP errors are `{"error": "..."}` strings (`server.rs:ErrorResponse`),
not rule codes — server-side additions need no code allocation. Client-side
package-trust diagnostics live in `6-605-0001..0009` (`src/rules/table.rs:1016-1069`,
ending `REGISTRY_LOG_ROLLBACK`); **`6-605-0010` is the next free** and is
plan-48-B's to take.

## 3. Design Overview

Three pieces:

1. **Generalize the blob store** — a `BlobKind` (`Package` | `Native`) threaded
   through `blob_name`, plus a `kind` column on `package_blobs`, so a native blob
   is stored and served honestly.
2. **The write half of the blob API** — `PUT /blob/<hash>` (session-authenticated,
   hash-verified, idempotent) and `HEAD /blob/<hash>` (the dedup probe), reusing
   the existing `exists`/`stage`/`promote`/`abort` protocol unchanged.
3. **Referential integrity at publish** — parse section 10, require every `vendor`
   hash to already exist, and record the version→blob edges.

### 3.1 Why individual uploads, not one multipart request

Rejected alternative: send the `.mfp` and every vendor file as parts of one
`multipart/form-data` request. Rejected for four independent reasons.

**It is more work, not less.** The instinct is that per-blob uploads mean
"building out support" while multipart is a single request. The opposite is true
here: multipart needs a new dependency (§2.3) *and* a body format foreign to a
protocol where, in the spec's own words, `post_json` is "the single transport
helper" and every payload is base64url-in-JSON. A raw `PUT /blob/<hash>` reuses
`exists`/`stage`/`promote` — machinery that **already exists** and is already
exercised by publish.

**No dedup.** `exists(hash)` (`blobstore.rs:172`) already answers "do you have
this?" A 40 MB `libsqlite3.so` unchanged across ten versions uploads **once**.
Multipart re-sends every byte on every publish, forever.

**The atomicity it appears to buy is a mirage.** A 240 MB request that dies at
90% yields nothing and restarts at zero; six PUTs that die at #5 resume at #5.
Real atomicity comes from **ordering**, not from one request: blobs first, `.mfp`
last. The `.mfp` is the commit point because it is the thing that references
blobs by hash — blobs without a `.mfp` are unreferenced garbage, not a broken
package, and §4.5's publish-time check guarantees the converse can never happen.
This is exactly the OCI/Docker registry model (`PUT` each blob, `PUT` the
manifest last).

**It does not compose with S3.** `GET /blob` already 302s so bytes never transit
the server. Only a raw PUT can eventually be presigned the same way (§4.6); a
multipart form cannot.

Also rejected: extend `PackageArtifactRequest` with a
`vendorBlobs: [{hash, bytes}]` base64 array. Zero new transport, but it re-sends
every blob on every publish, adds 33% base64 overhead to the payload that can
least afford it, and multiplies the ~48 MiB effective ceiling problem instead of
fixing it.

### 3.2 Why native blobs need no signature of their own

The `.mfp` signature does **not** cover the whole file — it covers
`signed_prefix = bytes[..signature_offset]`, the header (`package.rs:152`,
`verify_package_signature` `:231-239`). The payload is welded on by a
hash-in-signed-header indirection: `packageBinaryHash` is a raw 32-byte field
*inside* the signed prefix, and `verify_payload_hash` (`package.rs:243-248`)
checks it equals `sha256(payload)`. Sections live inside the payload. So:

```
signature ──covers──> signed_prefix (header)
                        └── packageBinaryHash (32 bytes)
                              └──equals──> sha256(packageBinaryRepr payload)
                                             └── section 10 + its vendor sha256s
```

**Vendor blob hashes are therefore transitively authenticated by the package
signature**, and a blob fetched by one of those hashes needs no signature —
re-hashing it is sufficient. This is not a novel argument: the registry already
reasons exactly this way about the ABI index (`server.rs:2010-2013` — "it is
covered by packageBinaryHash + the signature, so the registry serves it for
resolution without having to trust it"). Vendor hashes inherit the guarantee by
the same reasoning, which is why this plan changes **nothing** in the
proof/attestation payloads.

**The load-bearing caveat:** this holds only where `verify_payload_hash` is
actually called. A valid signature over a swapped payload passes the *signature*
check and fails only the hash weld — the two must always be paired. They are, at
both call sites (`src/cli/build.rs:1225-1230`, `server.rs:1992-1997`). Any new
code path that verifies a signature without the weld silently voids this whole
argument.

**Second caveat: unsigned packages get zero authentication**
(`src/cli/build.rs:1160-1162` short-circuits to `Unsigned` before any check). A
section-10 hash in an unsigned `.mfp` is attacker-controlled, so its vendor blob
is trust-on-first-use at best. plan-48-B must not present an unsigned package's
vendor blob as verified.

## 4. Detailed Design

### 4.1 `BlobKind` and honest blob names

```rust
pub enum BlobKind { Package, Native }
```

`blob_name(hash, kind)` → `<hash>.mfp` for `Package` (unchanged — **no migration
of existing blobs**), `<hash>.bin` for `Native`. Thread `kind` through
`blob_ref`, `exists`, `stage`, `get`, and the S3 `key()`.

`package_blobs` gains a `kind TEXT NOT NULL DEFAULT 'package'` column, so
existing rows migrate implicitly and `GET /blob/<hash>` can learn a blob's kind
from a primary-key lookup before touching the backend. That lookup is a bonus,
not just a cost: it lets an unknown hash 404 from SQLite without an S3 round
trip.

Rejected: storing native blobs as `<hash>.mfp` anyway (zero work, but the
filename lies to every operator who lists the datapath). Rejected: dropping the
suffix entirely (cleanest namespace, but requires renaming/copying every existing
object in every deployed registry — a real operator burden for a cosmetic win).

### 4.2 `HEAD /blob/<hash>` — the dedup probe

`200` if a servable blob exists, `404` otherwise. Same 64-lowercase-hex
validation as GET (`server.rs:1318-1326`), no body, no auth (it reveals only
whether a *content hash the caller already knows* is present — the same
information `GET` already gives, and the existing design already leans on hashes
being unguessable, `blobstore.rs:21`).

### 4.3 `PUT /blob/<hash>` — the write half

- **Auth:** required. Unlike GET, this writes. The session JWT travels in a JSON
  body field everywhere else on this server (`sessionToken`, `server.rs:448`),
  which a raw-body PUT cannot do — so carry it in an `Authorization: Bearer
  <token>` header for this one route and verify with the existing
  `verify_session_token` (`server.rs:2032-2046`), which already checks `exp`
  *and* that the `jti` session row is still live. **This is the protocol's first
  header-borne credential**; call it out in the spec rather than letting it be
  discovered.
- **Body:** raw bytes, `application/octet-stream`. Subject to the existing 64 MiB
  `DefaultBodyLimit` (`server.rs:666`).
- **Verification before storage:** compute `sha256(body)`; if it does not equal
  the `<hash>` in the path, `400` and store nothing. The blob store is
  content-addressed, so this is the invariant that keeps it honest.
- **Idempotent:** if `exists(hash, Native)` → `200` without re-staging. Racing
  PUTs of identical bytes are harmless (same content, same key).
- **Storage:** `stage(hash, kind, bytes)` → insert the `package_blobs` row →
  `promote(staged)`, with `abort` on failure — the exact
  order publish already uses (`server.rs:1806-1841`), preserving the
  "no servable orphan" invariant (`blobstore.rs:16-23`).
- **Rate limiting:** add a per-owner blob-upload limit alongside the existing
  `VALIDATE_PER_OWNER_MAX` / `PUBLISH_PER_OWNER_MAX` (`server.rs:606-607`).
  Without one, an authenticated publisher can fill the datapath with 64 MiB
  objects that **nothing can ever reclaim** (§2.4). This is not optional.

An uploaded-but-unreferenced blob is an orphan. That is the accepted trade of
the ordering model (§3.1) and the reason the Open Decisions call for a GC plan.

### 4.4 Referential integrity at publish

`POST /publish` (`server.rs:1762`) gains, after the existing validation and
before the stage/commit/promote block:

1. decode the artifact (already done, `server.rs:1799`) and parse its section-10
   `NATIVE_LIBRARY_TABLE`;
2. for every `vendor` locator, `exists(hash, Native)`;
3. if any is missing → `400`, naming the missing hashes and the logical library,
   and publish nothing.

This is what makes "blobs first, `.mfp` last" a guarantee rather than a
convention: after a successful publish, every section-10 hash resolves. The
server needs no new trust to do this — section 10 is inside the payload it has
already welded and verified (§3.2).

The server already decodes the payload to build the ABI index, so section-10
decoding is not new capability; reuse plan-46-B's decoder rather than
hand-rolling a second parser in the registry crate.

### 4.5 Recording version→blob edges

```sql
CREATE TABLE package_version_blobs (
  package_version_id INTEGER NOT NULL REFERENCES package_versions(id),
  hash TEXT NOT NULL REFERENCES package_blobs(hash),
  PRIMARY KEY (package_version_id, hash)
);
```

Written inside the publish transaction. **Nothing reads it in this plan** — the
client learns its blob list from section 10, not from the registry. It exists so
a future GC can compute reachability, and so an operator can answer "what does
this version pull in?" without parsing `.mfp`s. Adding it now costs one table and
means the data is not lost for every package published before GC exists.

### 4.6 Deliberately deferred: streaming and presigned PUT

Both are real, both are out of scope, both should be recorded so the ceiling is a
known limit rather than a surprise:

- **64 MiB cap.** Raw PUT already improves on base64's ~48 MiB effective ceiling,
  and covers the overwhelming majority of real native libraries (`libsqlite3` is
  ~1-2 MB). A genuinely large library (a bundled ML runtime) will hit it. The fix
  is streaming the body to the backend instead of buffering, plus a per-route
  limit override — not simply raising `MAX_BODY_BYTES`, which would let a 256 MiB
  JSON publish buffer ~3× in RAM (`server.rs:590-592`).
- **Presigned PUT** would let the client upload straight to S3, bypassing the
  server as `GET` already does. It costs the server-side hash verification (§4.3),
  since nothing inspects the bytes before they land. Defensible — the consumer
  re-verifies against the signed section-10 hash regardless — but it trades a
  cheap, certain check for a bandwidth win the current scale does not need.

## Compatibility / Format Impact

- **Protocol:** two new routes (`PUT`/`HEAD /blob/<hash>`); `POST /publish` gains
  a rejection case. No existing request or response shape changes.
- **Database:** additive — `package_blobs.kind` (defaulted, so existing rows
  migrate implicitly) and a new `package_version_blobs` table.
- **Blob storage:** existing `<hash>.mfp` objects are untouched and keep their
  names. Native blobs use a new `<hash>.bin` namespace. **No migration.**
- **Trust model:** unchanged. No new signatures, no proof/attestation change
  (§3.2).
- **Older clients** are unaffected: they never PUT, and a package with no
  section-10 vendor locators publishes and downloads exactly as today.

## Phases

### Phase 1 — `BlobKind` + schema, no new routes

Pure refactor; behavior-identical. Safe to land alone.

- [ ] Add `BlobKind { Package, Native }`; change `blob_name(hash)` →
      `blob_name(hash, kind)` (`blobstore.rs:136`) and thread `kind` through
      `blob_ref`, `exists`, `stage`, `get`, and S3 `key()` (`blobstore.rs:300`).
- [ ] Add `kind TEXT NOT NULL DEFAULT 'package'` to `package_blobs`
      (`store.rs:235-239`); confirm existing rows read back as `Package`.
- [ ] Add `package_version_blobs` per §4.5 (written in Phase 3).
- [ ] Tests: `<hash>.mfp` naming for `Package` is **byte-for-byte unchanged**
      (the existing `blob_ref_and_name_are_content_addressed` test at
      `blobstore.rs:462-466` must pass untouched); `Native` yields `<hash>.bin`;
      a pre-existing DB migrates and serves.

Acceptance: an existing registry's blobs and DB keep working with no migration
step and no name change; `cargo test -p mfb_repository` green, including the s3
feature build (`cargo build -p mfb_repository --features s3`).
Commit: —

### Phase 2 — `PUT` / `HEAD /blob/<hash>`

- [ ] Add `HEAD /blob/:hash` per §4.2 (same hex validation as
      `server.rs:1318-1326`).
- [ ] Add `PUT /blob/:hash` per §4.3: `Authorization: Bearer` session auth,
      raw body, `sha256(body) == <hash>` or `400`, idempotent on `exists`, then
      stage → row → promote with `abort` on failure.
- [ ] Add a per-owner blob-upload rate limit beside `PUBLISH_PER_OWNER_MAX`
      (`server.rs:606-607`) — §4.3, non-optional given there is no GC.
- [ ] Tests: round-trip a native blob PUT → HEAD 200 → GET returns identical
      bytes; wrong-hash body → 400 and **nothing stored**; re-PUT is 200 and does
      not duplicate; missing/expired/revoked token → 401; oversized body → 413;
      non-hex path → 400. Cover the S3 backend too (the `abort`-leaves-no-orphan
      test at `blobstore.rs:468-505` is the model).

Acceptance: a native blob uploads, HEADs, and downloads byte-identically on both
the local and S3 backends; a hash-mismatched upload stores nothing; an
unauthenticated PUT is refused.
Commit: —

### Phase 3 — publish referential integrity

- [ ] In `publish_package` (`server.rs:1762`), parse section 10 from the decoded
      artifact and require `exists(hash, Native)` for every `vendor` locator;
      `400` naming the missing hashes otherwise (§4.4). Reuse plan-46-B's
      section-10 decoder — do not hand-roll a second parser.
- [ ] Write `package_version_blobs` rows inside the publish transaction (§4.5).
- [ ] Apply the same check in `POST /validate` (`server.rs:1753`) so a dry run
      reports missing blobs before the publisher uploads anything.
- [ ] Tests: publishing a `.mfp` with a section-10 vendor hash whose blob is
      absent → 400, and **nothing is published**; uploading the blob then
      publishing → success, with the `package_version_blobs` edges recorded; a
      package with no vendor locators publishes exactly as before (regression).
- [ ] Doc: extend `src/docs/spec/package-manager/01_repository-protocol.md` —
      the endpoint table (`:59-91`) gains `PUT`/`HEAD /blob/<hash>`; document the
      `Authorization: Bearer` credential as the one header-borne exception to the
      body-field `sessionToken` convention (§4.3); document the publish rejection;
      note the `<hash>.bin` vs `<hash>.mfp` namespace.

Acceptance: a section-10 hash can never dangle after a successful publish,
verified by the negative test; existing packages publish byte-identically.
Commit: —

## Validation Plan

- Tests: per phase above; `cargo test -p mfb_repository` plus a `--features s3`
  build and the S3-backend blob tests.
- Runtime proof: run `mfb-repo` locally against a temp datapath, `PUT` a real
  multi-MB file, `HEAD` it, `GET` it back, and diff the bytes; repeat against a
  MinIO instance for the S3 path (`--s3-endpoint`) to prove the presigned 302
  serves a `.bin` blob as happily as a `.mfp`.
- Doc sync: `src/docs/spec/package-manager/01_repository-protocol.md`;
  `.ai/specifications.md` obligation.
- Acceptance: `scripts/test-accept.sh` green (the registry has its own tests in
  `repository/`; `tests/repo_acceptance.rs` covers the CLI-facing surface).

## Open Decisions

- **Garbage collection — resolved: filed as plan-49.** The registry has never
  deleted a blob (§2.4), which was tolerable when blobs were small `.mfp` files.
  Native libraries are orders of magnitude larger, ×7 target slots, and §4.3's
  orphan case means bytes can be stored that **nothing ever references**. This
  plan adds `package_version_blobs` (§4.5) so the reachability data exists and a
  rate limit so the bleeding is bounded; **plan-49** consumes that table to mark
  reachable blobs from live versions and sweep the rest after a grace period.
  plan-49 is `package_version_blobs`'s first and only reader — which is why §4.5
  writes it now rather than leaving the edges unrecorded for every package
  published before GC exists.
- Whether `HEAD /blob/<hash>` should require auth. Recommend no: it discloses only
  whether a content hash the caller already possesses is present, which `GET`
  already reveals, and the design already relies on hashes being unguessable
  (`blobstore.rs:21`). Revisit if hash-probing is ever considered an oracle.
- Whether `/validate` should return the **list** of missing blob hashes so the
  client uploads exactly what is needed in one round trip, making per-blob `HEAD`
  probes unnecessary. Recommend adding the list (the server has the parsed table
  in hand anyway) but keeping `HEAD` as the stateless primitive; plan-48-B can
  use either.

## Summary

The registry turns out to be well-shaped for this: blobs are **already**
content-addressed by SHA-256, `exists`/`stage`/`promote`/`abort` is **already**
the right protocol, and `GET /blob/<hash>` **already** serves arbitrary bytes with
an S3 presigned path. The work is the missing write half, an honest name for
non-`.mfp` blobs, and a publish-time check that section-10 hashes resolve.

Individual uploads are not the expensive option — multipart is (§3.1). And the
trust story needs no new crypto: section 10 rides inside the payload that
`packageBinaryHash` already welds to the signature, so a vendor blob is
authenticated by re-hashing alone (§3.2) — the same argument the registry already
makes for the ABI index.

The one thing this plan makes worse is storage: large, permanent, now
orphan-able blobs in a store with no GC. That is called out, bounded by a rate
limit, and left with the reachability rows a GC will need.

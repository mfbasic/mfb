# Audit 1 — Package Registry Service & plan-10 Design

Code-grounded security review of the MFBASIC package registry service
(`repository/src/*`), the client integration (`src/cli/repo.rs`, `src/cli/pkg.rs`),
and the plan-10 design docs. The service is an Axum HTTP server backed by a single
`Mutex<Connection>` SQLite store, with Ed25519 proof-of-possession registration,
challenge/response login, HS256 JWT sessions, and signed-`.mfp` publish.

Scope note: findings labeled **(design gap)** are documented in plan-10 as
not-yet-implemented and are called out to confirm the current *coded* posture, not
to fault the plan. "Already-coded bug/weakness" findings are live in the current
source. Severities weigh the fact that today `mfb-repo` binds `127.0.0.1:7777` by
default (localhost) — several issues escalate sharply the moment it is exposed
(a `--listen 0.0.0.0:...` away).

---

## REPO-01 — HIGH: Publish writes the blob to disk before authorization/ownership is committed (TOCTOU + orphan/DoS)

**Location:** `repository/src/server.rs:300-341` (`publish_package`)

**Issue:** The publish handler writes the artifact to the content-addressed blob
path *before* it re-verifies the session and hands off to the store's
ownership-checking transaction:

```rust
let artifact = crypto::decode_bytes(&request.artifact, "artifact")...;
let hash = report.content_hash;
let path = state.packages_dir.join(format!("{hash}.mfp"));
let blob_stored = !path.exists();
if blob_stored {
    std::fs::write(&path, &artifact)          // <-- disk write happens here
        .map_err(...)?;
}
let owner_id = verify_session_token(&state.store, &request.session_token)
    .map_err(bad_request)?
    .owner_id;
let published = state.store.publish_package_version(owner_id, ...)?;  // <-- DB commit / ownership check here
```

Consequences grounded in the code:
1. **Orphan blobs / write-amplification DoS.** If `publish_package_version` fails
   (e.g. `package_id` lookup returns `ok_or_else("package identity is owned by
   another owner")` at `store.rs:433`, or the version already exists, or the tx
   fails), the blob is already on disk and is never cleaned up. This exactly
   matches the plan-10 §2.6 note "Blob written to disk before the DB row commits …
   a failed transaction leaves an orphan blob." Because the file is
   content-addressed by attacker-chosen content, an attacker can write an unbounded
   number of distinct `<hash>.mfp` files to the packages directory by submitting
   packages that pass `validate_package_request` (their *own* packages) but then
   deliberately collide/fail the version insert, or simply by publishing many
   distinct byte-sequences — filling the disk.
2. **Content confusion via `INSERT OR IGNORE` on the blob row.** `publish_package_version`
   uses `INSERT OR IGNORE INTO package_blobs (hash, path, ...)` (`store.rs:434-439`).
   The `blob_stored = !path.exists()` guard means: if a blob for `<hash>` already
   exists on disk, `publish` will *not* rewrite it, will silently reuse whatever is
   there, and reports `blobStored: false`. Combined with the content hash being the
   trusted key, this is benign only as long as the on-disk bytes are guaranteed to
   equal `hash` — which is never re-verified on read (see REPO-08).

**Trigger:** `POST /publish` with a valid session for owner A and a valid,
self-consistent `.mfp` whose version already exists (or otherwise trips the
version `UNIQUE` at `store.rs:441`). Repeat with distinct payloads to accrete
orphan blobs. curl-style:
```
POST /publish {ident:"alice#pkg", version:"1.0.0", artifact:<valid mfp>, ...}
# second call with same version -> "already published", but the blob was re-attempted / left
```

**Fix:** Do all authorization first (verify session + resolve `owner_id`), then run
the DB transaction, and write the blob only *after* the transaction commits — or
write to a temp file and `fs::rename` into place on commit (atomic, no orphan on
rollback). On any DB failure after a write, delete the just-written blob. This is
the plan-10-A Phase A2 "Fix the publish ordering bug" item; it should be treated as
a live bug, not just a planned improvement.

---

## REPO-02 — HIGH: No request-size cap on inline base64 artifacts / bodies (memory-exhaustion DoS)

**Location:** `repository/src/server.rs:146-159` (router build — no
`DefaultBodyLimit` layer), `server.rs:292-348` (`validate`/`publish` decode the
whole base64 artifact into memory), `Cargo.toml` (`axum = "0.7"`)

**Issue:** The router is built with no body-size limit layer:

```rust
let app = Router::new()
    .route("/publish", post(publish_package))
    ...
    .with_state(state);
```

Axum's `Json<T>` extractor with the default configuration will buffer the entire
request body into memory before deserializing. `PackageArtifactRequest.artifact` is
a base64 `String` that is then fully `crypto::decode_bytes`'d into a second `Vec<u8>`
(`server.rs:311`, `348`). There is no `DefaultBodyLimit`, no streaming, and no
upfront length check. plan-10 §2.6 explicitly lists "No request-size cap on inline
base64 artifacts" as an open gap (design gap for the *cap*, but the memory-blowup is
a live exposure).

**Trigger:** `POST /publish` (or `/validate`, which needs only a session but not a
valid package) with a multi-gigabyte base64 `artifact` field. The server allocates
the raw body + the decoded bytes + the parse working set, repeated across concurrent
requests, exhausting memory. `/validate` is the cheapest vector because it returns a
report rather than requiring a committed publish.

**Fix:** Add `.layer(DefaultBodyLimit::max(N))` to the router (e.g. a few MiB, sized
to the largest legitimate `.mfp`). Additionally validate `request.artifact.len()`
before decoding and reject oversized packages early. Consider the plan-10-D2
"upload-reference flow" for genuinely large blobs.

---

## REPO-03 — HIGH: No rate limiting on register / challenge / login (brute-force + resource DoS)

**Location:** `repository/src/server.rs:151-159` (no rate-limit/tower-governor
layer); `register` (177), `challenge` (193), `login` (218)

**Issue:** None of the auth endpoints are rate limited. `POST /auth/challenge`
inserts a fresh 32-byte nonce row into `auth_challenges` on every call for any known
owner (`store.rs:259-285`) with a 5-minute TTL and **no reaping** (plan-10 §2.6: "No
expired-challenge / expired-session reaping"). `POST /accounts/register` runs an
Ed25519 verify + a transaction per call. Every request also contends the single
global `Mutex<Connection>` (see REPO-09).

Consequences:
- Unbounded growth of `auth_challenges` / `sessions` rows (no reaper).
- CPU burn via forced Ed25519 verifications on `register`.
- Owner-enumeration oracle: `/auth/challenge` returns `bad_request("unknown owner")`
  (`server.rs:202`) vs. issuing a challenge, letting an attacker enumerate valid
  owner handles.

**Trigger:** A loop of `POST /auth/challenge {owner:"guess"}` (enumeration + row
growth) or `POST /accounts/register` (CPU + row growth). No credential needed.

**Fix:** Add per-IP + per-owner rate limiting (e.g. `tower_governor`) on
register/challenge/login; add a background reaper for expired challenges and
sessions; make the "unknown owner" response indistinguishable in timing/shape from a
successful challenge issuance (or gate challenge issuance behind a generic 200). This
is plan-10-D2 (rate limiting + reaping) — currently entirely absent.

---

## REPO-04 — MEDIUM: JWT `aud`/`iss` unbound and validation relies on defaults; session secret is per-repo random (tokens survive restart, but no rotation/kill-switch)

**Location:** `repository/src/server.rs:435-449` (`verify_session_token`),
`store.rs:474-490` (`ensure_server_secret`), `store.rs:378-382` (`server_secret`)

**Issue:** Positives first: the secret is a fresh 32 random bytes generated once and
persisted in the `server_secrets` table (`store.rs:481-488`) — it is **not**
hardcoded and **not** re-randomized per boot, so it is a real HS256 secret and tokens
survive restart. `alg` is pinned via `Validation::new(Algorithm::HS256)` (so
`alg=none`/RS256-confusion is rejected by jsonwebtoken v9, which checks the header
`alg` against the allowed set). `validate_exp = true` is set explicitly, and every
protected endpoint additionally checks `store.session_exists(jti)` against a DB row
(`server.rs:445`), so a revoked/deleted session row invalidates the token — a genuine
server-side kill switch.

Residual weaknesses:
1. **No `aud`/`iss` binding.** Claims carry `sub/owner_id/auth_fingerprint/iat/exp/jti`
   only (`server.rs:136-144`); `Validation` sets no `aud`/`iss`. jsonwebtoken v9's
   default `Validation` for HS256 does **not** require `aud`, so this is accepted, but
   the token is not scoped to this service — a secret shared/reused across
   environments would make tokens cross-usable. Low impact today, worth hardening.
2. **No leeway/clock-skew consideration and a 1h fixed TTL with no refresh/rotation**
   (`server.rs:227-228`). Acceptable, noted.
3. **Secret has no rotation path.** If the DB `server_secrets` row leaks, every
   outstanding session is forgeable until each `jti` row is individually removed; there
   is no versioned-secret or global-invalidation mechanism.

**Trigger:** N/A for a direct exploit given the DB-backed `jti` check; this is a
defense-in-depth / token-scoping gap.

**Fix:** Add an `iss` and `aud` to `SessionClaims` and require them in
`Validation` (`validation.set_issuer`, `validation.set_audience`). Keep the DB
`jti` check (it is the strongest control here). Consider a secret-version column to
allow rotation with staged invalidation.

---

## REPO-05 — MEDIUM: `validate`/`publish` trust request-supplied fields, but `validate` requires only a session (info + validation-oracle for any authenticated user)

**Location:** `repository/src/server.rs:292-298` (`validate_package`),
`343-424` (`validate_package_request`)

**Issue:** `/validate` requires only a valid session token
(`verify_session_token` at `server.rs:347`) — it does **not** require the session
owner to match the package's owner before doing the full parse + hash + signature
work and returning detailed diagnostics. The owner mismatch is only added as a
*diagnostic string* (`server.rs:377-379`: "session owner does not match package
ident owner") rather than a hard authorization failure, and the endpoint still
returns the recomputed `contentHash` and a granular diagnostics list. While
`/publish` re-checks and refuses when `!report.valid` (`server.rs:305-310`), an
authenticated attacker can use `/validate` as an oracle to:
- Recompute/confirm content hashes for arbitrary `.mfp` bytes.
- Probe another owner's current key state indirectly (each mismatch yields a specific
  diagnostic naming which of ident-key/ident-fingerprint/signing-fingerprint/author
  disagreed — `server.rs:391-403`).

This is not a privilege escalation (publish is still gated), but it is an
information-disclosure oracle available to any registered user, combined with no rate
limit (REPO-03).

**Trigger:** `POST /validate` with any valid session and a crafted
`PackageArtifactRequest` targeting another owner's ident, reading the diagnostics.

**Fix:** Reject at `/validate` when `fold_owner(owner_part) != fold_owner(claims.sub)`
(hard 403) before emitting per-field diagnostics, and collapse the per-field key
mismatches into a single non-descriptive failure for cross-owner requests. Rate-limit
per REPO-03.

---

## REPO-06 — MEDIUM: Local key/session files use the un-sanitized owner string as a filename (path traversal on the client)

**Location:** `repository/src/local.rs:34-45` (`public_key_path`,
`private_key_path`, `session_path`), consumed by `client.rs` and `src/cli/repo.rs`

**Issue:** File paths are built by direct interpolation of `owner`:

```rust
pub fn public_key_path(&self, owner: &str) -> PathBuf {
    self.keys_dir().join(format!("{owner}.pub"))
}
pub fn session_path(&self, owner: &str) -> PathBuf {
    self.session_dir().join(format!("{owner}.ses"))
}
```

`validate_owner_name` (`validation.rs:7-36`) restricts owners to
`[A-Za-z_][A-Za-z0-9_]*`, so the *normal* client paths (`register`, `auth`) call it
first (`client.rs:18`, `39`, `95`, `121`, `137`) and are safe. However
`LocalPaths` is a public API with no internal guard: any caller that reaches
`write_keypair`/`write_session`/`read_session` with an owner that bypassed
validation (e.g. `../../.ssh/authorized_keys`) would write/read outside the keys
dir. The safety here is entirely dependent on every caller pre-validating.

**Trigger:** A client code path (current or future) that constructs `LocalPaths`
paths from an owner value not passed through `validate_owner_name` — e.g. an owner
read from a package ident (`ident.split_once('#')`) rather than from the CLI arg.

**Fix:** Call `validate_owner_name(owner)` (or a stricter filename-safe check) inside
`LocalPaths::*_path` or at the top of `write_keypair`/`write_session`/`read_*`, so the
filesystem layer is self-defending rather than relying on caller discipline.

---

## REPO-07 — MEDIUM: Blob filename is derived from a server-recomputed hash, but the hash charset/length is never asserted before filesystem use

**Location:** `repository/src/server.rs:312-313` (`publish_package`),
`package.rs:116-136` (`package_content_hash`)

**Issue:** `let path = state.packages_dir.join(format!("{hash}.mfp"));` where
`hash = report.content_hash`. In the current flow `hash` comes from
`package.content_hash_hex()` → `hex::encode(self.content_hash)` over a `[u8; 32]`
(`package.rs:30-32`, `100`), so it is always 64 lowercase hex chars — filesystem-safe.
That is the *only* reason there is no path traversal here today. There is no explicit
assertion that `hash` matches `^[0-9a-f]{64}$` before it is used as a filename, so the
safety is implicit in "the value came from `hex::encode`". The request-supplied
`request.content_hash` is *not* used for the path (good — it is only compared, at
`server.rs:357`), which is the correct design. This is defense-in-depth: today safe,
but brittle if a future refactor ever routes a client-controlled hash into the path.

**Trigger:** N/A today (server recomputes the hash). Would become live if any
client-supplied hash string were ever used to build the blob path.

**Fix:** Assert the hash is exactly 64 hex chars (or reconstruct the filename from the
raw `[u8;32]` via `hex::encode`) immediately before `packages_dir.join(...)`, so the
filesystem call can never receive `/`, `.`, or `..`.

---

## REPO-08 — MEDIUM: `GET /blob/<hash>` and on-read hash verification do not exist (design gap) — served blobs are never re-verified against their hash

**Location:** design gap — `planning/plan-10-A-keys-install.md:44-52` (Phase A2,
unchecked); confirmed absent from `repository/src/server.rs:151-159` (no `/blob`
route)

**Issue (design gap, confirmed against code):** There is currently no install/serve
path at all — `/blob/<hash>` and `/index/<owner>#<package>` are not routed. plan-10-A
Phase A2 specifies that `/blob` must "verify on read that the recomputed hash matches
the path (blob-store corruption defense)." Until implemented, note that: (a) blobs are
written once and never re-hashed (REPO-01's `INSERT OR IGNORE` reuse means a blob on
disk is trusted by content-address alone), and (b) the plan correctly calls for
on-read verification — that requirement must survive into the implementation.

**Trigger:** N/A (endpoint not implemented). Flagged so the on-read verify requirement
is not dropped, and so the REPO-01 reuse behavior is fixed before `/blob` ships.

**Fix:** When implementing Phase A2, recompute `package_content_hash` on every
`GET /blob/<hash>` read and 500/404 on mismatch; make `/blob` reject any `<hash>` that
is not `^[0-9a-f]{64}$` before touching the filesystem (mirrors REPO-07).

---

## REPO-09 — MEDIUM: Single global `Mutex<Connection>` serializes all requests → trivial DoS amplifier and lock-poisoning cliff

**Location:** `repository/src/store.rs:12-14` (`Arc<Mutex<Connection>>`), every store
method locks it (e.g. `store.rs:100`, `189`, `231`, `269`, `288`, `348`, `366`, `379`,
`415`)

**Issue:** All database access — including the multi-statement publish transaction
(`store.rs:415-453`) and the Ed25519-verifying registration transaction
(`store.rs:189-214`) — is serialized behind one mutex. Combined with no rate limiting
(REPO-03) and no body cap (REPO-02), a single slow/large request holding the lock
stalls the entire service. Additionally every lock site maps a poisoned lock to
`"database lock poisoned"`; if any handler panics while holding the lock (e.g. an
unexpected `rusqlite` state), the mutex is poisoned and **every** subsequent request
fails permanently until restart. plan-10 §2.6 / plan-10-D2 flag this as the "global DB
mutex bottleneck."

**Trigger:** Concurrency against any endpoint while a large `/validate` or `/publish`
(REPO-02) holds the lock; or induce a panic in a locked section to poison the mutex.

**Fix:** plan-10-D2: move to SQLite WAL with a small connection pool or a dedicated
writer task; avoid holding the lock across CPU-heavy crypto (do the Ed25519 verify
before acquiring the lock — currently `register_owner` verifies *before* locking,
which is good, but `complete_challenge` verifies the signature *inside* the held
transaction at `store.rs:334-335`). Recover from poisoning (`into_inner()`) rather
than failing forever.

---

## REPO-10 — LOW: `logEntry` is a fake random string — no transparency log exists yet (design gap)

**Location:** `repository/src/server.rs:339` (`log_entry: format!("publish:{}",
Uuid::new_v4())`); design gap tracked in `planning/plan-10-C-transparency-trust.md:29-42`

**Issue (design gap):** The publish response advertises a `logEntry`, but it is a
random UUID with no backing append-only Merkle log, no inclusion proof, and no signed
checkpoint. Any client that treats `logEntry` as evidence of transparency-log
inclusion is trusting a value the server can fabricate arbitrarily. This matches
plan-10-C Phase C1 (unimplemented). The security consequence: the registry provides
**no** tamper-evidence or auditability today — a compromised or malicious registry can
serve different bytes to different clients with no cryptographic detection.

**Trigger:** N/A (design state). Flagged so `logEntry` is not consumed as a trust
signal before C1 lands.

**Fix:** Implement plan-10-C Phase C1 (RFC 6962 Merkle log + `/log/checkpoint` +
inclusion/consistency proofs); until then, clients must not treat `logEntry` as
inclusion evidence.

---

## REPO-11 — LOW: Three-role key model is aliased — a single auth key acts as ident + signing key (design gap, current security consequence)

**Location:** `repository/src/server.rs:262-290` (`signing_info` returns the same
`public_key` as both `ident_key`/`signing_key` and the same `key.fingerprint` as both
fingerprints); `store.rs:206-211` registers a single `role='auth'` key; design in
`planning/plan-10-A-keys-install.md:27-39` (Phase A1, unchecked)

**Issue (design gap, current consequence):** `repository.md` §2–§3 require three
independent Ed25519 roles (auth/ident/signing) with separate rotation/revocation. As
coded, registration stores one `auth` key and `/keys/signing` returns it in all three
slots:

```rust
let public_key = crypto::encode_bytes(&key.public_key);
Ok(Json(SigningInfoResponse {
    ident_key: public_key.clone(),
    ident_fingerprint: key.fingerprint.clone(),
    signing_key: public_key,
    signing_fingerprint: key.fingerprint,
}))
```

Security consequence today: there is **no separation of duties**. The same private
key that authenticates the account (proves login) also signs packages. Compromise of
the single key = full account takeover *and* the ability to sign malicious releases;
there is no lower-privilege signing key that can be rotated without re-proving account
ownership, and no ability to revoke signing capability independently of login. Package
verification (`validate_package_request`, `server.rs:391-404`) checks ident and
signing fingerprints against the *same* key, so the distinct-authority guarantee the
spec promises does not exist.

**Trigger:** N/A (architectural). Consequence realized on any single-key compromise.

**Fix:** Implement plan-10-A Phase A1 (three keys, role-discriminated proofs so a
proof for one role cannot be replayed as another — note `crypto::registration_message`
at `crypto.rs:66-73` currently has no role discriminator, so cross-role proof replay
would be possible the moment multiple roles share that message format; add the
discriminator when splitting roles).

---

## Checked and OK

- **SQL injection: none found.** Every query in `store.rs` uses `rusqlite` bound
  parameters (`params![...]` / `[]`) — `register_owner` (193, 206), `owner_with_auth_key`
  (232), `create_challenge` (270), `complete_challenge` (293, 336), `insert_session`
  (349), `session_exists` (368), `server_secret` (380), `count_owners` (386),
  `package_version_exists` (392), `publish_package_version` (419, 425, 434, 440),
  `force_expire_challenge` (466), `ensure_server_secret` (477, 483). No string-formatted
  SQL anywhere. The schema DDL (`migrate`, 101-172) is a static literal.
- **Ed25519 signature verification is done over exact bytes with the registered key.**
  `verify_package_signature` (`package.rs:104-114`) rebuilds the message from the
  server-recomputed `content_hash` + `ident` + `version` (domain-separated with
  `MFP-PACKAGE-v1`) and verifies against `key.public_key` loaded from the DB
  (`server.rs:404`) — not any client-supplied key. `verify` (`crypto.rs:35-48`) enforces
  exact 32-byte pubkey / 64-byte signature lengths.
- **Content hash is recomputed server-side and compared.** `package_content_hash`
  (`package.rs:116-136`) is SHA-256 over header + zeroed-signature region + body (canonical:
  the signature bytes are replaced with zeros so signing is well-defined), full 32 bytes,
  no truncation. The server ignores `request.content_hash` for the blob path and only
  *compares* it (`server.rs:357`). The request `ident`/`version`/fingerprints are all
  cross-checked against the parsed package (`server.rs:357-371`).
- **Proof-of-possession at registration is enforced.** `register_owner` verifies the
  proof over `registration_message(owner, public_key)` against the submitted key before
  insert (`store.rs:182-184`); a package cannot be published under an owner without the
  owner having proven key possession, and publish binds the ident owner to the session
  owner (`server.rs:373-390`) and the store binds the package identity row to the session
  `owner_id` (`store.rs:425-433`, rejecting "package identity is owned by another owner").
- **Challenge/response is single-use and expiring.** `complete_challenge` rejects
  reused (`used_at IS NOT NULL`, `store.rs:328`) and expired (`store.rs:331`) challenges,
  marks used inside the transaction with an idempotent `WHERE used_at IS NULL`
  (`store.rs:337`), and the nonce is 32 CSPRNG bytes (`store.rs:265-266`,
  `rand::thread_rng().fill_bytes`). Login binds the JWT to a DB `jti` row so a token is
  revocable and replay after deletion fails.
- **JWT `alg` is pinned to HS256** (`server.rs:437`, `Validation::new(Algorithm::HS256)`),
  so `alg=none` and algorithm-confusion are rejected by jsonwebtoken v9. `exp` is validated
  and `jti` presence in `sessions` is checked on every protected call (`server.rs:445`).
- **Server secret is CSPRNG (32 bytes), generated once, persisted** — not hardcoded, not
  re-randomized per boot (`store.rs:474-490`). Tokens survive restart.
- **Local key/session files are written with tight perms** — dirs `0o700`, files `0o600`
  on Unix (`local.rs:87-102`); private key never leaves disk unencrypted-in-transit (only
  the public key + signatures go over the wire in `client.rs`).
- **`.mfp` parser is bounds-checked** — every offset advance uses `checked_add` and
  compares against `bytes.len()`; string fields are length-capped
  (`package.rs:64-81`, `156-207`); signature type/length are validated
  (`validate_signature_header`, 147-154). No unchecked slice indexing on attacker input.
- **Owner-name validation is strict** (`validation.rs:7-36`): ASCII-only,
  `[A-Za-z_][A-Za-z0-9_]*`, length-capped at 255, `std` reserved, case-folded for
  uniqueness (`fold_owner`). This is what keeps REPO-06/REPO-07 latent rather than live on
  the normal paths.

---

## Severity summary

- **HIGH (3):** REPO-01 publish blob-before-commit TOCTOU/orphan/DoS; REPO-02 no
  body-size cap (memory DoS); REPO-03 no rate limiting / owner-enumeration on auth
  endpoints.
- **MEDIUM (6):** REPO-04 JWT unbound `aud`/`iss` + no secret rotation; REPO-05
  `/validate` cross-owner oracle; REPO-06 owner-as-filename path-traversal latent on the
  client; REPO-07 blob filename hash not asserted safe-charset; REPO-08 `/blob` on-read
  verify not implemented (design gap); REPO-09 global `Mutex<Connection>` DoS amplifier +
  poisoning.
- **LOW (2):** REPO-10 fake `logEntry` / no transparency log (design gap); REPO-11
  aliased three-role key model — no separation of duties (design gap, live consequence).

Live already-coded issues to prioritize: **REPO-01, REPO-02, REPO-03, REPO-09**
(all exploitable without special privilege the moment the service is exposed off
localhost). The rest are hardening or tracked plan-10 design gaps.

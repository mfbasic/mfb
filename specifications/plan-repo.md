# MFBASIC Package Registry Completion Plan

Last updated: 2026-06-21

This document scopes the work required to take the repository service from its
current base (registration, authentication, single-key publish) to the full
design in `specifications/repository.md`. It is the successor planning document
to `specifications/repo-base.md`, which deliberately limited the first pass to
account registration and authentication.

It complements:

- `specifications/repository.md` — the full registry design (authoritative)
- `specifications/repo-base.md` — the completed base pass
- `specifications/package_format.md` — `.mfp` container + content hash
- `specifications/project.md` — `project.json` + `mfb pkg` CLI surface
- `specifications/lockfile.md` — `mfb.lock` schema

This is a planning document. It records what exists today, the gaps against the
full design, and a phased implementation sequence with validation criteria for
each phase.

---

## 1. Summary

The base pass shipped a working but narrow slice:

- `mfb-repo --path <repo>` HTTP service with `meta.db` (SQLite) + `packages/`.
- `POST /accounts/register`, `POST /auth/challenge`, `POST /auth/login`,
  `GET /health`, plus `POST /keys/signing`, `POST /validate`, `POST /publish`.
- Ed25519 proof-of-possession registration, challenge/response login, HS256 JWT
  sessions (1h, tracked by `jti`).
- Signed-package publish that parses the `.mfp` header, recomputes the content
  hash, verifies the signature, enforces ident/version/owner binding, writes the
  content-addressed blob, and records `packages` / `package_versions` /
  `package_blobs`.
- Client CLI: `mfb repo register|auth`, `mfb pkg publish`, `mfb build --sign`,
  `mfb pkg add` (today only `file://…​.mfp`), `mfb pkg info|verify`.

The largest structural shortcut is the **key model**. `repository.md` §2–§3
require three distinct Ed25519 roles per owner — **auth**, **ident**, and
**signing** — with independent rotation and revocation. The implementation
registers a single `auth` key and `POST /keys/signing` returns that same key
in all three roles. Almost every signing, rotation, transparency, and
verification feature depends on splitting these three roles first.

The second-largest gap is that **nothing can be installed**. There is no
`GET /blob/<hash>`, no `GET /index/<owner>#<package>`, no resolver, and no
`mfb.lock`. A package can be published but never resolved or downloaded through
the registry.

The third is that **trust infrastructure is stubbed**. `logEntry` is a random
`format!("publish:{uuid}")` string; there is no transparency log, no Merkle
tree, no inclusion/consistency proofs, and no signed root/snapshot/timestamp
metadata.

The phases below are ordered so each builds on the last and each ends at a
testable milestone.

---

## 2. Gap Analysis

Organized against `repository.md`. Status: ✅ done · ◑ partial · ❌ missing.

### 2.1 Identity & naming (§1)

| Item | Status | Notes |
|---|---|---|
| Owner-handle validation, `std` reserved | ✅ | `validation.rs` |
| `<owner>#<package>` ident on publish | ◑ | Parsed/enforced at publish; no package-slug vs header-`name` separation |
| `std#*` resolution short-circuit | ❌ | Resolver does not exist |
| Identity permanence / visible tombstones | ❌ | No tombstone records; deletion path undefined |
| Typosquat warn-only check at publish | ❌ | |
| `mfb pkg add <owner>#pkg[@ver][--pin]`, name search | ❌ | `add` only accepts `file://` |
| `mfb pkg install` (lock-driven) | ❌ | |

### 2.2 Accounts (§2)

| Item | Status | Notes |
|---|---|---|
| Open key-based registration | ✅ | auth key only |
| Three key roles (auth/ident/signing) | ❌ | Single `auth` key; `/keys/signing` aliases it |
| Orgs (members, owner/admin/publisher roles) | ❌ | |
| Key-rotation recovery | ❌ | |
| Publish tokens (delegated CI creds, TTL, revoke) | ❌ | |
| Ownership transfer (two-sided, signed, logged) | ❌ | |

### 2.3 Trust & signing (§3)

| Item | Status | Notes |
|---|---|---|
| Proof-of-possession at registration | ◑ | One proof; spec wants three |
| Distinct ident vs signing authority | ❌ | |
| `POST /keys/rotate` + `past`/`revoked` lifecycle | ❌ | Columns exist, unused |
| Revocation endpoint | ❌ | |
| Past-key install verification (`publishedAt < rotatedAt`) | ❌ | |
| Transparency log (Merkle, checkpoints) | ❌ | `logEntry` is a fake string |
| Inclusion + consistency proofs | ❌ | |
| Offline root + `/root.json` | ❌ | |
| Online snapshot/timestamp + `/snapshot.json` `/timestamp.json` | ❌ | |
| `signatureType=1` enforced; unsigned rejected | ✅ | `package.rs` |

### 2.4 Resolution & lockfile (§4, §8)

| Item | Status | Notes |
|---|---|---|
| `ABI_INDEX` computed by compiler, embedded in `.mfp` | ❌ | publish returns `abiIndex: {}` |
| `ABI_INDEX` stored + served in index | ❌ | |
| Per-symbol superset substitution resolver | ❌ | |
| Diamond conflict diagnostics | ❌ | |
| `mfb.lock` write + byte-identical re-resolve | ❌ | |
| `mfb pkg check-abi` | ❌ | |

### 2.5 Registry API & storage (§5)

| Endpoint | Status |
|---|---|
| `/accounts/register` `/auth/challenge` `/auth/login` `/health` | ✅ |
| `/validate` `/publish` | ◑ (single-key; no log/ABI) |
| `/keys/rotate` | ❌ |
| `/index/<owner>#<package>` | ❌ |
| `/blob/<hash>` | ❌ |
| `/release-state` | ❌ |
| `/root.json` `/snapshot.json` `/timestamp.json` | ❌ |
| `/log/checkpoint` `/log/proof/<entry>` | ❌ |
| Release states `deprecated`/`yanked`/`blocked`/`legal-tombstoned` | ❌ (only `available`) |

### 2.6 Operational gaps (not in spec, needed for correctness)

- Single `Mutex<Connection>` serializes all DB access; no WAL, no pool.
- No expired-challenge / expired-session reaping.
- No rate limiting on registration, challenge, or login.
- Blob written to disk *before* the DB row commits in `publish_package`; a
  failed transaction leaves an orphan blob.
- Session and challenge verification run twice per publish (minor).
- No request-size cap on inline base64 artifacts; no upload-reference path.

---

## 3. Implementation Phases

Each phase is independently shippable and ends with the listed tests green.

### Phase 1 — Three-role key model (foundation)

Everything in §3 of the spec assumes auth, ident, and signing are separate keys.
Do this first; later phases depend on the schema and the registration shape.

Work:

1. Extend `POST /accounts/register` to accept `authKey`, `identKey`,
   `signingKey` and `proofs.{auth,ident,signing}`, each proof signed over the
   registration challenge for its own key (`crypto::registration_message` gains a
   role discriminator so a proof for one role cannot be replayed for another).
2. `Store::register_owner` inserts three `keys` rows (`role` ∈
   `auth|ident|signing`, all `status='current'`). Return all three fingerprints.
3. Replace `POST /keys/signing` aliasing: add `Store::current_key(owner, role)`
   and have `/validate`/`/publish` check `identFingerprint` against the current
   **ident** key and the release signature against the current **signing** key.
4. Update `RegisterResponse` to `{authFingerprint, identFingerprint,
   signingFingerprint, logEntry}` per §5.
5. Client: `mfb repo register` generates and stores three keypairs
   (`<owner>.auth.{pub,prv}`, `<owner>.ident.{pub,prv}`,
   `<owner>.signing.{pub,prv}`); `mfb build --sign` signs with the signing key;
   ident metadata in the `.mfp` uses the ident key/fingerprint.

> Decision needed: local key-file naming changes the on-disk layout from
> `repo-base.md` §2. Either migrate existing `<owner>.prv` → `<owner>.auth.prv`
> or keep `<owner>.{pub,prv}` as the auth key and add the two new files. Default
> recommendation: keep auth at the existing path, add `.ident`/`.signing`
> siblings — no migration of already-registered local state.

*Decision* - key names are <owner>.<type>.[prv|pub]

Tests: registration persists three keys; publish verifies ident-vs-signing
correctly; a package signed by the ident key (not signing key) is rejected;
acceptance test publishes end-to-end with the new key set.

### Phase 2 — Install path: `/blob` and `/index`

Makes published packages retrievable. No new trust primitives yet.

Work:

1. `GET /blob/<hash>` — stream `packages/<hash>.mfp`; 404 if absent; immutable,
   long-cache headers. Verify on read that the file's recomputed hash matches the
   path (defense against blob-store corruption).
2. `GET /index/<owner>#<package>` (`#`→`%23`) — return the §5 version list:
   `version, hash, publishedAt, state, identFingerprint, signingFingerprint,
   signingKeyStatus, signingKeyRotatedAt, abiIndex, logEntry`. Add
   `Store::list_package_versions`.
3. Fix the publish ordering bug: write the blob inside (or after) the committed
   transaction, or write to a temp file and rename only on commit.
4. Client `mfb pkg add <owner>#pkg[@ver]`: hit `/index`, pick latest (or the
   requested) version, `GET /blob/<hash>`, verify hash + signature locally,
   install into the project's package dir, and append to `project.json`.
   Keep the existing `file://` path as a `source: "file:"` special case.

Tests: publish then `GET /blob/<hash>` returns identical bytes; `GET /index`
lists the version; `add <owner>#pkg` installs and verifies; tampered blob is
rejected on the hash check.

### Phase 3 — ABI index

Unlocks real resolution (Phase 5) and `check-abi`.

Work:

1. Compiler: compute `ABI_INDEX` — one hash per exported symbol over its full
   public shape (functions, records, unions, enums, constants, globals, native
   wrappers, resource behavior, effect flags) per §8.2. Embed it as a new `.mfp`
   metadata section (extend `package_format.md`).
2. `repository/src/package.rs` parses the section; `/validate` and `/publish`
   return the real `abiIndex` and `Store` persists it on the version row.
3. `/index` serves the stored `abiIndex`.
4. `mfb pkg check-abi` diffs the working tree's `ABI_INDEX` against the latest
   published version and names changed/dropped symbols.

Tests: golden ABI hashes are stable across rebuilds; adding an export is a
superset; changing a signature changes exactly that symbol's hash; `check-abi`
names the changed symbol.

### Phase 4 — Transparency log

Replace the fake `logEntry` with a real append-only log. Required before the
signed-metadata layer is meaningful.

Work:

1. New `log_entries` table: monotonic index, entry kind (registration,
   key-rotation, revocation, publish, release-state-change, ownership-transfer),
   payload, leaf hash, timestamp. Build an in-DB Merkle tree (RFC 6962 hashing).
2. Append an entry from every state-changing endpoint; return the real
   `logEntry` (index + leaf hash).
3. `GET /log/checkpoint` — signed tree head (size + root hash). `GET
   /log/proof/<entry>` — inclusion proof; support consistency proofs between two
   tree sizes.
4. Client pins the last-seen checkpoint and rejects rollback.

Tests: inclusion proof verifies against the checkpoint root; consistency proof
verifies across appends; tampering with a leaf breaks the root.

### Phase 5 — Resolver + lockfile

Work:

1. Implement the §8.3 algorithm in the client: single-dep latest-compatible
   (`ABI_INDEX(V) ⊇ ABI_INDEX(anchor)`), diamond union with precise conflict
   diagnostics naming requirers and disagreeing symbols, exact `--pin` bypass.
2. Honor release-state eligibility (`available`/`deprecated` eligible, `yanked`
   pin-only, `blocked`/`legal-tombstoned` excluded).
3. Write `mfb.lock` per `lockfile.md` (selected/requested versions, hashes, key
   metadata, ABI metadata, checkpoint, root/snapshot/timestamp versions).
4. `mfb pkg install`: with a current lock, fetch by hash only — never resolve.
   `mfb pkg update`: explicit re-resolution producing a reviewable lock diff.

Tests: re-resolve is byte-identical; diamond conflict names both requirers; a
patch release is selected as a compatible substitute; locked install does no
index lookups.

### Phase 6 — Key rotation, revocation, release states

Work:

1. `POST /keys/rotate` (§5): authenticated, signed by current ident key, makes
   replacements `current`, old keys `past`, logged. Enforce past-key publish
   rejection.
2. Revocation endpoint: immediate stop on new auth/publish from the key, logged.
3. Install-time past-key verification: a `past`-signed package verifies only if
   `publishedAt < signingKeyRotatedAt`; surface
   `Verified with old signing key rotated on …`.
4. `POST /release-state` (§5): maintainer sets `available|deprecated|yanked`
   (not `blocked`/`legal-tombstoned`), signed, logged, blob untouched.

Tests: rotation moves keys; old key can't publish; a package published before
rotation still verifies, after rotation does not; yanked excluded from floating
resolution but selectable by pin.

### Phase 7 — Signed metadata root-of-trust

Work:

1. Offline registry root key + `root.json` binding registry ID, delegated
   snapshot/timestamp keys, thresholds, expiration, root version.
2. Online snapshot/timestamp keys; `GET /snapshot.json`, `GET /timestamp.json`
   carrying index hashes, versions, expiry, and the log checkpoint reference.
3. Client trust: configured registry ID + pinned root fingerprint; reject
   expired metadata, undelegated keys, registry-ID mismatch, version rollback,
   and any index entry whose signature chain or publish inclusion proof fails.

Tests: tampered snapshot rejected; expired timestamp rejected; rollback to an
older snapshot version rejected; first-install verifies the full chain.

### Phase 8 — Accounts: orgs, publish tokens, transfers

Lowest urgency; closes §2.

Work: org handles with member/role tables; CI publish tokens (owner/package
scoped, short TTL, revocable, never bypassing ident/signing checks); two-sided
signed ownership transfers. All logged to the transparency log.

### Phase 9 — Hardening

- SQLite WAL + a small connection pool (or a writer task) to remove the global
  mutex bottleneck.
- Background reaping of expired challenges and sessions.
- Rate limiting on register/challenge/login.
- Request-size cap on inline artifacts; optional upload-reference flow for large
  `.mfp` blobs.
- Typosquat warn-only check at publish.

---

## 4. Suggested Sequencing

```
Phase 1 (keys) ─┬─> Phase 2 (blob/index) ─┬─> Phase 5 (resolver/lock)
                │                          │
                └─> Phase 3 (ABI) ─────────┘
Phase 4 (log) ──> Phase 6 (rotation/states) ──> Phase 7 (signed metadata)
Phase 8 (accounts) and Phase 9 (hardening) interleave as capacity allows.
```

Phases 1–2 deliver a registry you can publish to *and install from*. Phases 3+5
make resolution real. Phases 4+6+7 deliver the security model that distinguishes
this registry from npm/PyPI. Phase 1 is a hard prerequisite for everything in
§3 and must land first.

## 5. Cross-Cutting Decisions To Confirm

1. **Local key file layout** for three roles (Phase 1) — recommend additive
   `.ident`/`.signing` siblings, no migration.
2. **ABI section placement** in the `.mfp` container — needs a
   `package_format.md` amendment (Phase 3) before the compiler emits it.
3. **Merkle hashing scheme** — recommend RFC 6962 to match CT/Rekor tooling
   expectations (Phase 4).
4. **Where root/snapshot/timestamp private keys live** — offline root must not
   sit on the serving host (Phase 7); document the signing workflow.

## 6. Out Of Scope (future passes, per `repository.md`)

- Threshold / N-of-M multi-sig for critical-tier packages (spec defers to v2).
- Federation / mirror discovery protocol.
- Exact legal-tombstone audit procedure and public takedown fields.
- Critical-package threshold values (download/dependent counts).

# Audit 2 — Surface 7: Package registry HTTP service (auth / transparency log / TUF metadata / blobs)

Last updated: 2026-07-14
Untrusted party: any remote registry client (anonymous or holding a scoped token).
Must not: publish into/transfer another owner's namespace, forge auth
challenges/logins/tokens, forge transparency-log or TUF metadata, poison the blob
store, or DoS/disk-fill the service.

Scope read: `repository/src/{server,store,crypto,log,package,abi,blobstore,local,
validation}.rs`, `docker-entrypoint.sh`. Dep usage audited (not internals): axum
0.7.9, ed25519-dalek 2.2.0, jsonwebtoken 9.3.1, rusqlite 0.31.0, p256 0.13.2,
security-framework 3.7.0, rustls 0.23.41.

## Authorization model — verified sound

Every account mutation goes through `verify_session_token` (HS256 JWT whose `jti`
must exist unrevoked in `sessions`, `server.rs:1950`) and/or `session_and_ident`
(session names `owner`, session `auth_fingerprint` is a *current* auth key, returns
the owner's ident key for an additional ident-signature check, `server.rs:888`).
Traced each mutating route — the two-credential design holds; **no cross-owner
bypass found**:
- **keys/rotate** (`:1324`): session + `chainSignature` under the *old ident*
  (`store.rs:710`) + PoP under the new key. No finding.
- **packages/transfer/offer** (`:1039`) / **accept** (`:1065`): offer requires
  session+ident of `from_owner` and re-checks current DB owner == `from_owner`
  (`store.rs:1451`); accept requires session+ident of `to_owner` matching a pending
  offer addressed to them (`store.rs:1517`). No finding.
- **tokens / tokens/revoke** (`:974/:1010`): session+ident, ident-signed,
  `scope_owner_matches` (`:1091`); scope enforced at `/signing` via `scope_permits`
  (`:1102,1638`). A publish-token holder is blocked at the ident-signature check
  (lacks the ident private key) — no escalation. No finding.
- **orgs/members** (`:916`), **release-state** (`:1208`), **signing** (`:1602`):
  session+ident + owner match / role authority; every signing issuance logged
  before signing (`store.rs:466`). No finding.

Functional-only (not security): after a transfer the package `ident` keeps the old
owner name, so `validate_package_request`'s `owner_part == session.sub` check
(`:1841`) blocks *both* old and new owner from publishing new versions to a
transferred ident. Flagged so it isn't mistaken for a vuln.

## Verdict on prior audit-1 findings (re-verified)

| ID | Prior sev | Verdict | Evidence |
|----|-----------|---------|----------|
| REPO-01 | HIGH | **FIXED** | stage→commit-DB→promote/abort (`server.rs:1733-1777`, `blobstore.rs:183-231`); bytes validated before staging. Residual LOW: a promote failure *after* the DB commit (`:1773`) leaves a version row with no servable blob (inconsistency, not orphan/disk-fill). |
| REPO-02 | MEDIUM | **MITIGATED** | `DefaultBodyLimit::max(64 MiB)` (`:592/647`). Residual aggregate DoS → REPO-13. |
| REPO-03 | MEDIUM | **PARTIAL** | limiter exists but register/login use global buckets (→ REPO-12); `/auth/challenge` still distinguishes unknown-owner (`:715`) and `/accounts/register` returns "already in use" — enumeration oracles remain (LOW). |
| REPO-04 | LOW | **STILL OPEN** | `Validation::new(HS256)` sets no `aud`/`iss` (`:1952`); server secret has no rotation path (`store.rs:1889`). Forgery still needs the secret + a live `jti` → LOW. |
| REPO-05 | MEDIUM | **FIXED** | `/validate` requires a session and enforces `fold(package owner)==fold(session.sub)` (`:1841`). Residual authenticated existence oracle (`:1856`) — LOW. |
| REPO-06 | LOW | **MITIGATED** | `validate_owner_name` restricts owner to `[A-Za-z0-9_]`; `local.rs` is client-side. Latent only. |
| REPO-07 | LOW | **FIXED** | `package_blob` asserts 64 lowercase-hex before FS use (`:1283-1289`). |
| REPO-08 | LOW | **FIXED** | local backend re-hashes served bytes (`:1303`); S3 re-hashes client-side (`blobstore.rs:29-36`). |
| REPO-09 | MEDIUM | **STILL OPEN** | `Arc<Mutex<Connection>>` serializes all DB access incl. reads (`store.rs:13`); a panic in a critical section permanently poisons it → "database lock poisoned" until restart. Triggering a panic is hard (rusqlite returns errors) → practical severity ~LOW-MEDIUM; architecture unchanged. |
| REPO-10 | LOW | **FIXED** | real RFC-6962 Merkle tree with inclusion + consistency proofs + server-signed checkpoint (`log.rs`), dense monotonic `idx` (`store.rs:1948`). Split-view caveat → REPO-19. |
| REPO-11 | LOW | **FIXED** | distinct `auth` (per-machine) and `ident` (account) keys with role-tagged PoP (`ROLE_AUTH`/`ROLE_IDENT`, `crypto.rs:69-81`); a test proves role-replay is rejected. |

## New findings

### REPO-12 — MEDIUM — Global (non-per-client) rate-limit buckets on register/login enable trivial lockout DoS
- Location: `server.rs:681` (`allow("register", 60, 60)`), `:1531`
  (`allow("login", 60, 60)`); limiter `:31-50`.
- Threat/impact: any anonymous client can deny registration and login to the
  *entire* user base for the window.
- Mechanism: both buckets are keyed by a **constant string**, not client identity,
  and the check runs before validation — 60 hits/60s total across all clients.
  `/auth/challenge` (`:704`) and `/signing` (`:1606`) are correctly per-owner;
  register/login are not.
- Reproduction: `for i in $(seq 1 61); do curl -sX POST $REPO/auth/login -d '{...}'; done`
  → the 61st and every legit login for the rest of the window return 429. Invalid
  attempts count (rate check precedes signature decode).
- Best fix: key the bucket by client identity (peer IP via `ConnectInfo`/
  `X-Forwarded-For`), or make register/login per-owner; keep the shared bucket
  only as a secondary global ceiling. Small fix → documented here (paired with
  bug-188).
- Non-goals: distributed rate limiting (a fronting proxy is assumed).

### REPO-13 — MEDIUM — No rate limit or quota on /validate and /publish → CPU/disk exhaustion
- Location: `server.rs:1712` (`validate_package`), `:1720` (`publish_package`);
  body cap `:592/647`; blob persisted `blobstore.rs:200`; log grows `store.rs:1115`.
- Threat/impact: registration is open, so "authenticated" is near-anonymous; a
  registered client hammers `/validate` (parse + 5 Ed25519 verifies/call) for CPU
  and `/publish` (permanent ~48 MiB blobs + unbounded log growth) for disk/DB.
- Mechanism: the limiter is applied only to register/challenge/login/signing;
  `/validate` and `/publish` have only the 64 MiB body cap — no per-call rate cap,
  no per-owner quota. Residual of REPO-02.
- Best fix: per-owner sliding-window on both routes; per-owner blob-bytes/version
  quota; smaller `/validate` body cap. → **bug-188**.

### REPO-14 — LOW — `publish_log_entry` uses SQL `LIKE` with un-escaped user-controlled ident/version → wrong log-entry match
- Location: `store.rs:1811-1840` (`WHERE kind='publish' AND payload LIKE ?1 || '%'`),
  prefix built `:1817` from `json_value(ident)`/`json_value(version)`.
- Threat/impact: the `logEntry` (index + leaf hash) surfaced by `/index/<ident>`
  (`server.rs:843`) and `/log/publish` (`:806`) can resolve to a *different*
  package's entry, corrupting the inclusion-proof mapping a client verifies.
- Mechanism: `_`/`%` are `LIKE` wildcards, unescaped (no `ESCAPE`). Owner names
  allow `_` (`validation.rs:28`) and the package/version components are unrestricted
  (REPO-17), so `a_b#pkg` matches `axb#pkg`. `json_value` escapes quotes, not LIKE
  metacharacters.
- Best fix: store `ident`/`version` as indexed columns and match by equality, or
  add `ESCAPE` and escape `_`/`%`/`\`.

### REPO-15 — LOW (not fully demonstrated) — /machines/link/fetch attaches an auth key gated only by `lookup`
- Location: `server.rs:1436` (`link_fetch`); `take_pairing_blob` (`store.rs:601`);
  `add_auth_key` (`store.rs:644`).
- Threat/impact: whoever presents a valid pending `lookup` gets an attacker-chosen
  auth key registered on that account (a login/session foothold) without the
  pairing code. Under TLS the `lookup` is confidential and the code unguessable, so
  not demonstrable without a `lookup` leak / TLS-strip; the design asymmetry
  (auth-key attachment protected only by `lookup=sha256(code)`, weaker than the
  ident-blob confidentiality) is the finding. The rogue key cannot publish/rotate
  and is revocable → bounded impact.
- Best fix: require the fetcher to prove code knowledge for the auth-key
  attachment too (HMAC/tag over the request keyed by a second code-derived value).

### REPO-16 — LOW — Uncached full-tree / full-index recomputation on every anonymous read → CPU amplification
- Location: `server.rs:729/752/780` (checkpoint/inclusion/consistency),
  `:1137/1169` (snapshot/timestamp); `log_leaf_hashes` (`store.rs:1788`),
  `index_canonical_hash` (`store.rs:1651`). Each cheap anonymous GET recomputes the
  RFC-6962 root O(n) or scans+sorts every `package_versions` row; no caching, no
  read rate limit. With REPO-13's unbounded growth, per-request cost is unbounded.
- Best fix: memoize root/checkpoint/index-hash, invalidate on append; light
  anonymous read rate limit.

### REPO-17 — LOW — Missing charset/length validation on the package component of an ident and on version
- Location: ident split without package-part validation (`server.rs:1232/1647/1837`,
  `package.rs:264/297`); version only length-checked (`≤64`, `package.rs:121`);
  owner validated but package/version are free-form UTF-8. Control chars, `#`,
  quotes, and LIKE wildcards flow into log payload / `/index` / the REPO-14 pattern.
  Best fix: restrict package + version to an explicit safe charset/length at parse
  and publish.

### REPO-18 — NTH (design note) — TUF is 1-of-1 threshold; online snapshot/timestamp keys share the serving DB
- Root private key is correctly returned offline (never persisted); but no
  signature threshold (>1 key), and a host/DB compromise yields
  snapshot+timestamp+attestation signing keys (not root) — `init_registry_root`
  (`store.rs:1557`), keys in `registry_config` (`store.rs:284-287`), used
  `server.rs:1156/1185`. Metadata is regenerated with `now`-based expiry per
  request (`:1151/1181`), so rollback detection relies on monotonic
  `version=log_size` + the client's stored snapshot-version.

### REPO-19 — NTH (design note) — Transparency log has no witness/gossip → split-view undetectable
- `log.rs` math is correct/well-tested; the checkpoint is signed only by the
  server's own key (`server.rs:734`). A malicious registry can present
  consistent-but-divergent views to different clients. Inherent to a
  single-operator log without external witnesses; noted for completeness (relates
  to SUP-03 downgrade defense on the client side).

## Verdict

The plan-10/plan-23 rework closed the serious audit-1 items (REPO-01/05/07/08/10/11)
and the core authz (rotate/transfer/tokens/orgs/signing) is sound with **no
cross-owner bypass, forgery, or namespace-takeover demonstrable**. Remaining
exposure is availability-centric: REPO-12 (global-bucket lockout, MEDIUM, small
fix) and REPO-13 (unthrottled validate+publish, MEDIUM → **bug-188**) first, then
REPO-14 (LIKE log-lookup correctness, LOW) and the persistent single-mutex
fragility REPO-09 (LOW-MEDIUM). REPO-04 (JWT aud/iss) LOW still open. No CRITICAL/HIGH.

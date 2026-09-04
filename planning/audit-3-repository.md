# audit-3 — Surface 8: package registry HTTP service (`mfb-repo`)

Part of `planning/goal-08-platform-security-review.md`. Finding prefix `REPO-`
(auth/routes REPO-01..10; store/log/TUF REPO-50..60). Untrusted party: any remote
registry client, anonymous or token-holding.

**Verdict: 3 HIGH · 7 MEDIUM · 6 LOW · 2 NTH.** The three HIGHs are cross-owner /
cross-credential **authorization bypasses**, all three **reproduced live and
independently by the lead** against a locally-run `mfb-repo` built from this
worktree (harness checked in at `spikes/audit-3/repository-authz/`). This is the
highest-impact surface in audit-3. The registry's *cryptographic* core
(signatures, transparency-log inclusion/consistency, TUF rollback) holds; the
failures are in **who is allowed to invoke** a route, in **resource accounting**
(unbounded/unauthenticated body buffering, byte quotas), and in a
**world-readable database**.

## HIGH — authorization bypasses (all lead-reproduced live)

### REPO-01 — a scoped publish token self-escalates to a permanent unscoped auth key → **bug-492**

`/signing` consults a token's scope only via `publish_token_for_key(key.id)`
(`server.rs:2609`, `store.rs:2177`), keyed on the *specific* key id — a different
auth key of the same account returns `None` and the scope gate is skipped
entirely. `link_start` needs only a session (`server.rs:2352`) and `link_fetch`
takes no account credential (`server.rs:2402`), so a token session pairs a new
**unscoped** auth key *with itself*, then signs any package of the account —
**after the token is revoked**.

Lead repro (`cargo run --bin tokenesc -- orgB1`):
`token /signing orgB1#flagship -> 400` (scoped) but
`escalated key /signing orgB1#flagship -> 200` with a server-signed attestation
for `orgB1#flagship`, *after* `CI token revoked = true`. Re-opens the DEFERRED
audit-2 REPO-15 with a threat model that contradicts its "bounded impact"
rationale.

### REPO-02 — an auth key revokes every other auth key → account lockout → **bug-493**

`complete_challenge_with`'s loader query (`store.rs:1196`) has no `k.role = 'ident'`
and no `purpose` predicate, and `create_auth_challenge`/`create_ident_challenge`
write identical rows — so an `/auth/challenge` id satisfies
`/machines/revoke` when signed with the attacker's *auth* key, contradicting the
handler's own doc ("an auth session must NOT suffice", `server.rs:2441`).
`revoke_auth_key` (`store.rs:1115`) has no last-key guard, so an account can be
driven to zero current auth keys — unrecoverable without the operator reanchor.

Lead repro (`cargo run --bin revoke -- victimB1`): `ident key NEVER used`,
`revoke status = 200`, primary key no longer usable for login.

### REPO-03 — `/release-state` + `/signing` authorize on the ident prefix, not the owner → **bug-494**

Both routes compare the ident's `owner#` prefix via `fold_owner`
(`server.rs:2046`, `:2630`), and `set_release_state` resolves the row by ident
with no `owner_id` predicate (`store.rs:2544`). The prefix never moves when
ownership does, so after a transfer the **former** owner still yanks the package
and gets a signed attestation, while the **new** owner is refused both.

Lead repro (`cargo run --bin transfer -- t2b`): after transfer to `bobt2b`,
`former-owner yank -> 200`, `former-owner /signing -> 200`,
`current-owner unyank -> 400`.

## MEDIUM

### REPO-04 — unauthenticated 64 MiB body buffered before any auth check; no concurrency cap or timeout
`server.rs:900,2208,2882`. Reproduced: 132→774 MB RSS from 8 anonymous
connections. A cheap memory-exhaustion DoS by an anonymous peer.

### REPO-50 — registry SQLite DB created world-readable (0644) → **lead-confirmed**
`store.rs:147` (`Connection::open` with no mode). The DB holds the server signing
key, TUF online keys, and the JWT secret. Lead-verified: the DB the server
created is `-rw-r--r--`. On a shared host any local user reads the registry's
entire key material. Fix: create the DB file 0600 (and its journal/WAL
siblings), and the data dir 0700.

### REPO-51 — case-insensitive `LIKE` in `publish_log_entry` → inclusion-proof pointer can resolve to another package's entry
`store.rs:2694`. The wildcard metacharacters are escaped (verified) but SQLite
`LIKE` is ASCII-case-insensitive, so a prefix can still match a differently-cased
ident's log entry — a residual of audit-2 REPO-14. Agent demonstrated at the SQL
level; lead confirmed the `LIKE` path. Fix: match with `= ` / `GLOB` (case-
sensitive) or fold case consistently with the ident normalization.

### REPO-52 — publishing a hash already stored as a `native` blob writes a second `.mfp` copy GC can never reclaim
`server.rs:2802`. Storage-exhaustion / GC-evasion.

### REPO-55 — no total-bytes quota: `PUT /blob` is 120/min × 64 MiB; version quota counts rows, not bytes
`server.rs:2222,789`. One free account can pin ~625 GiB.

### REPO-56 — anonymous unthrottled `/log/*`, `/snapshot.json` recompute the whole tree/index; `/index/:ident` is O(V×N) full log scans
`server.rs:1000,1603`. Escalation of the deferred audit-2 REPO-16 — CPU
exhaustion by an anonymous client.

### REPO-06 — per-IP limits key on the TCP peer; behind the documented Fly proxy every client collapses into one bucket
`server.rs:829`, `fly.toml:39`. Re-opens REPO-12's effect in the deployed config.

### REPO-07 — quotas count rows/requests, never bytes (companion to REPO-55)
`server.rs:783,789,793`.

## LOW

- **REPO-05** — `/machines/revoke/challenge` has no rate limit (anonymous
  unbounded row insertion + owner oracle). `server.rs:2445`.
- **REPO-08** — rate-limiter map has no key cap and prunes on a 3600 s window vs
  60 s buckets → unbounded memory from distinct IPs. `server.rs:52,818`.
- **REPO-09** — publish-token expiry enforced only at `/signing`; an expired
  token still logs in and `PUT /blob` succeeds. `server.rs:2609`, `store.rs:2135`.
  (Demonstrated by `spikes/audit-3/repository-authz/ cargo run --bin expired`.)
- **REPO-10** — attestations carry no expiry and `/publish` never re-checks the
  token scope/expiry/revocation. `server.rs:2546`, `package.rs:278`.
- **REPO-53** — `BlobStore::get` never re-hashes; `backfill` ingests blob bytes
  with no signature/payload-hash check. `blobstore.rs:353`, `backfill.rs:64`.
- **REPO-54** — log append index from `COUNT(*)` not `MAX(idx)+1`; no hash chain
  between leaves (the Merkle tree is rebuilt, not chained). `store.rs:2916`.
- **REPO-57** — client private keys created at umask mode then chmodded 0600
  (race window). `local.rs:386` (same class as SUP-06).

## NTH

- **REPO-58** — `root.json` `version` never checked by clients; every
  `init-root` mints a new root key (no in-band rotation). `store.rs:2398`,
  `client.rs:838`.
- **REPO-59** — signed messages are NUL-separated, not length-prefixed (the
  domain tags are correct and distinct, so no concrete confusion found).
  `crypto.rs:126`.
- **REPO-60** — client widens a wire `i64` checkpoint size/index with `as usize`,
  no sign check. `client.rs:733`.

## Re-verified positive (recorded so a later audit does not re-derive)

- Transparency-log **inclusion** (`verify_inclusion` guards `index >= tree_size`,
  `log.rs:76`) and **consistency** (guards `m==n`, `m==0`, `m>n` before any `-1`)
  fail closed.
- Signature domain tags are distinct per message type (`crypto.rs`), so no
  cross-protocol signature reuse was found despite REPO-59's encoding nit.
- Owner names strictly validated before entering a path (`validation.rs:9`);
  `<owner>#<package>` percent-encoded in URLs (`client.rs:789`).
- No TLS bypass in the client (see `audit-3-supply-chain.md`).

## Bug docs filed

bug-492 (REPO-01), bug-493 (REPO-02), bug-494 (REPO-03) — the three HIGH
authorization bypasses. The MEDIUMs whose fix is larger than a line
(REPO-04/50/55/56) are recorded here for a follow-up; REPO-50 in particular is a
one-line-mode fix with high impact and is a strong candidate to file.

## Coverage

Auth pass (REPO-01..10): `repository/src/server.rs` read by route; `store.rs`
read along its auth/authz paths; `web/mod.rs` page-builders skimmed (escaping
argument structural + grep-verified); `main.rs`/`lib.rs`/`validation.rs` read.
Store pass (REPO-50..60): `crypto.rs`, `log.rs`, `blobstore.rs`, `store.rs` TUF/
blob paths, `gc.rs`/`backfill.rs` read; `abi.rs` read.

Gaps: `repository/src/web/**` not read in full (assumed lower-value HTML surface
— flag if a dedicated web-XSS pass is wanted); the `s3` blob backend was read but
not compiled/executed; `cargo test --manifest-path repository/Cargo.toml` was not
run to establish a green baseline (a fixer must). The lead independently built
`mfb-repo` and ran the three HIGH exploits live; REPO-50 was lead-confirmed by
inspecting the created DB's mode; the remaining MEDIUM/LOW/NTH are code-level with
quoted evidence.

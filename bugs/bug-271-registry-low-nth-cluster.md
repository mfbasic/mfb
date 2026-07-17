# bug-271: registry residual LOW/NTH cluster — JWT aud/iss, LIKE-wildcard log match, pairing auth-key gating, read amplification, charset, TUF/witness design notes

Last updated: 2026-07-17
Effort: large (3h–1d across items)
Severity: LOW
Class: Security / robustness / design-note

Status: Open
Regression Test: (none yet)

A batch of individually-LOW/NTH residual findings on the `mfb-repo` registry
surface from audit-2 that lack their own bug docs. The core authorization model
is sound (no cross-owner bypass/forgery/namespace takeover); these are hardening
and two design notes. Grouped per the repo's low-severity-batch convention; the
higher-severity registry items are bug-188 (REPO-12/13) and bug-264 (REPO-09).

References:

- `planning/audit-2-repository.md` (REPO-04, REPO-14, REPO-15, REPO-16, REPO-17,
  REPO-18, REPO-19).

## Findings

### REPO-14 — `publish_log_entry` uses SQL `LIKE` with un-escaped user-controlled ident/version
- Location: `repository/src/store.rs:1811-1840` (`WHERE kind='publish' AND payload
  LIKE ?1 || '%'`), prefix built `:1817` from `json_value(ident)/json_value(version)`.
- Symptom: `_`/`%` are unescaped `LIKE` wildcards (no `ESCAPE`); owner names allow
  `_` and package/version are unrestricted (REPO-17), so `a_b#pkg` matches
  `axb#pkg`. The `logEntry` (index+leaf hash) surfaced by `/index/<ident>` and
  `/log/publish` can resolve to a *different* package's entry, corrupting the
  inclusion-proof mapping a client verifies. **This is a correctness/integrity
  bug, the most actionable item here.**
- Fix: store `ident`/`version` as indexed columns and match by equality, or add
  `ESCAPE` and escape `_`/`%`/`\`.

### REPO-17 — missing charset/length validation on the package component of an ident and on version
- Location: ident split without package-part validation
  (`repository/src/server.rs:1232/1647/1837`, `package.rs:264/297`); version only
  length-checked (`≤64`, `package.rs:121`); owner validated but package/version are
  free-form UTF-8.
- Symptom: control chars, `#`, quotes, and LIKE wildcards flow into the log
  payload / `/index` / the REPO-14 pattern.
- Fix: restrict package + version to an explicit safe charset/length at parse and
  publish (fixing REPO-17 also shrinks the REPO-14 attack surface).

### REPO-15 — /machines/link/fetch attaches an auth key gated only by `lookup`
- Location: `repository/src/server.rs:1436` (`link_fetch`);
  `store.rs:601` (`take_pairing_blob`); `store.rs:644` (`add_auth_key`).
- Symptom: whoever presents a valid pending `lookup=sha256(code)` gets an
  attacker-chosen auth key registered on that account (a login/session foothold)
  without the pairing code. Under TLS the `lookup` is confidential and the code
  unguessable, so not demonstrable without a `lookup` leak / TLS-strip; the
  asymmetry (auth-key attachment protected only by `lookup`, weaker than the ident
  blob's confidentiality) is the finding. The rogue key cannot publish/rotate and
  is revocable → bounded.
- Fix: require the fetcher to prove code knowledge for the auth-key attachment too
  (an HMAC/tag over the request keyed by a second code-derived value).

### REPO-16 — uncached full-tree/full-index recomputation on every anonymous read
- Location: `repository/src/server.rs:729/752/780` (checkpoint/inclusion/
  consistency), `:1137/1169` (snapshot/timestamp); `store.rs:1788`
  (`log_leaf_hashes`), `store.rs:1651` (`index_canonical_hash`).
- Symptom: each cheap anonymous GET recomputes the RFC-6962 root O(n) or
  scans+sorts every `package_versions` row — no caching, no read rate limit. With
  unbounded log growth (REPO-13/bug-188) per-request cost is unbounded → CPU
  amplification DoS.
- Fix: memoize root/checkpoint/index-hash and invalidate on append; add a light
  anonymous-read rate limit. (Composes with the bug-264 connection-pool change.)

### REPO-04 — JWT sets no `aud`/`iss`; server secret has no rotation path
- Location: `repository/src/server.rs:1952` (`Validation::new(HS256)` — no
  `aud`/`iss`); `store.rs:1889` (server secret, no rotation).
- Symptom: forgery still requires the server secret *and* a live `jti`, so LOW; but
  the missing audience/issuer binding and un-rotatable secret are hardening gaps.
- Fix: set and validate `aud`/`iss` on issued/verified tokens; add a secret
  rotation path (versioned key id in the token header).

### REPO-18 — TUF is 1-of-1 threshold; online snapshot/timestamp keys share the serving DB (NTH, design note)
- Location: `repository/src/store.rs:1557` (`init_registry_root`),
  `store.rs:284-287` (keys in `registry_config`), used `server.rs:1156/1185`.
- Symptom: the root private key is correctly offline (never persisted), but there
  is no signature threshold (>1 key), and a host/DB compromise yields
  snapshot+timestamp+attestation signing keys (not root). Rollback detection relies
  on monotonic `version=log_size` + the client's stored snapshot-version. Design
  note; recorded for completeness.

### REPO-19 — transparency log has no witness/gossip → split-view undetectable (NTH, design note)
- Location: `repository/src/log.rs` (math correct/well-tested); checkpoint signed
  only by the server's own key (`server.rs:734`).
- Symptom: a malicious registry can present consistent-but-divergent views to
  different clients. Inherent to a single-operator log without external witnesses;
  relates to the client-side SUP-03 downgrade defense (bug-189). Design note.

## Goal

- REPO-14 log-lookup matches by equality/escaped LIKE (no cross-package match);
  REPO-17 package/version restricted to a safe charset/length; REPO-15 auth-key
  attachment proves code knowledge; REPO-16 tree/index results memoized + anon
  read limit; REPO-04 JWT carries validated `aud`/`iss` + a secret-rotation path.
  REPO-18/19 are recorded design notes (no code change expected without a
  multi-signer/witness design decision).

### Non-goals (must NOT change)

- The sound core authz model (rotate/transfer/tokens/orgs/signing).
- The on-disk schema in a way that breaks existing published data (REPO-14/17
  column changes must migrate, not drop).
- Introducing distributed rate limiting or external witnesses as a prerequisite
  (a fronting proxy / single-operator log is assumed).

# bug-276: repository-crate LOW cluster (durability ordering, fail-open pins, unbounded reads, dead code, timestamp/overflow edges)

Last updated: 2026-07-17
Effort: medium (1h–2h across items)
Severity: LOW
Class: Correctness / Security (defense-in-depth) / Dead-code / Footgun

Status: Open
Regression Test: per-item (see each)

A cluster of LOW-severity, mostly latent or defense-in-depth findings across the
`mfb-repo` registry crate, discovered during the goal-06 full source review. Each
is independently small; grouped here per the repo's low-cluster convention
(cf. bug-93/97/98/270). Distinct root causes, one document. None is a broken
security guarantee on its own; the common theme is fail-open / unbounded / stale
edges around otherwise-hardened code.

References:

- Found during goal-06 review of `repository/src/{client,server,store,abi,local,main}.rs`.
- Cross-checked against audit-1/audit-2 repository + supply-chain docs and
  bug-259..271; none of the below is a re-file (notes per item).

## Items

### R1 — `rotate_ident` persists the new ident key only after the server commits (durability ordering)
- `repository/src/client.rs:350-361` (`rotate_ident`).
- The new ident private key exists only in memory while `POST /keys/rotate` runs;
  the server marks the old ident `past` in that transaction.
  `local::write_ident_keypair` runs only on success — if that write fails (disk
  full/permissions) or the response is lost after the server committed (timeout
  mid-response), the new private key is dropped and the old one is no longer the
  account authority, potentially bricking the account (rotation exists precisely
  because other machines are lost/untrusted). Sibling of bug-272.
- Fix: write the new keypair to pending files (`<owner>.ident.next.{pub,prv}`)
  *before* the request; promote on success, and reconcile a leftover pending key
  against the server's current ident on startup/auth.
- Prior-work: new (audit-2 covers the protocol, not this ordering).

### R2 — `verify_log_consistency` is production-dead and pins the forked head before verifying
- `repository/src/client.rs:513-537` (`verify_log_consistency`), pin write at
  `client.rs:463` (`fetch_checkpoint`).
- No production caller (only its own unit test), so the fork-at-larger-size case
  is never audited: `fetch_checkpoint`'s monotonicity checks pass a forked log
  that simply grows. And when called, it invokes `fetch_checkpoint` (which
  `local::write_checkpoint`s the NEW head) *before* fetching/verifying the
  consistency proof, so a detected fork is already pinned — the fork evidence
  self-destructs after one error.
- Fix: fetch-without-pinning, verify the consistency proof against the candidate,
  then pin; and wire `verify_log_consistency` into the publish/resolve/verify
  flows that call `fetch_checkpoint`.
- Prior-work: new (REPO-19 covers multi-client split-view, not this pin ordering).

### R3 — no size cap on registry response bodies
- `repository/src/client.rs:1032-1035` (`fetch_blob` `response.bytes()`),
  `client.rs:1182-1200` (`read_json_response`), error-path `.text()` at
  `client.rs:1022`/`1095`.
- Every response body is read fully into memory with no length check; the
  SHA-256 check runs only after buffering. A hostile registry — or the S3-backend
  302 presigned-URL host that `fetch_blob` follows silently — can stream for up to
  `BLOB_TIMEOUT` (600 s), forcing a multi-GB allocation before any hash/parse
  rejection. Pure DoS; integrity is unaffected.
- Fix: honor `Content-Length` and read via a `take(limit)` chunked loop with a
  hard cap (≈64 MiB + slack for blobs, ≈1 MiB for JSON/error bodies).
- Prior-work: new (audit-2 notes fetch_blob is SHA-256-verified, not the missing
  size bound).

### R4 — S3 `abort` on a concurrent duplicate publish deletes the shared content-addressed object
- `repository/src/server.rs:2076-2089` (publish abort path);
  `repository/src/blobstore.rs:277-289` (S3 abort DELETEs by content key).
- For S3, staged and promoted keys are both the content hash. Two concurrent
  publishes of the same new `(ident, version)` share the same bytes/hash; the
  loser hits the unique-constraint conflict and `abort(staged)` DELETEs the object
  the winner just committed (winner's `promote` is a no-op), leaving a committed
  `package_versions` row whose blob 404s. Local backend unaffected (per-stage
  UUID temp files).
- Fix: make S3 `abort` a no-op for a key a committed row references (or only
  delete a key this request created), or serialize stage/promote per content hash.
- Prior-work: new (REPO-01 predates the S3 backend / content-keyed abort).

### R5 — native-blob PUT colliding an existing package blob orphans a `.bin` and mislabels kind
- `repository/src/server.rs:1543-1566` (`put_blob`); `store.rs:1247-1258`
  (`record_native_blob` `INSERT OR IGNORE`).
- `put_blob` checks `exists(hash, Native)` only. If a `package_blobs` row already
  exists for that hash with `kind='package'`, the `INSERT OR IGNORE` is ignored so
  the row keeps `kind='package'`, but `promote` still writes `<hash>.bin` — an
  orphan (no GC) plus a kind mismatch. Content correctness preserved (byte
  identical, GET re-hashes); storage waste + cosmetic mismatch.
- Fix: `record_native_blob` should detect an existing row of a different kind and
  refuse / skip the `.bin` promote, or key blob file+row by `(hash, kind)`.
- Prior-work: new (post-audit plan-48-A code).

### R6 — `log_leaf_hashes(size)` truncates on negative/zero size → wrong-view consistency proof
- `repository/src/store.rs:1925-1945` (`log_leaf_hashes`); callers
  `server.rs:841`/`870` pass unvalidated `Option<i64>` query params.
- `WHERE idx < ?1` with a negative `size` yields zero leaves; `log_consistency_proof`
  then computes `to = 0` and can produce a "valid" empty-range proof for a
  non-empty log — an integrity-signal weakening for a client given `to` in good
  faith via a rewriting proxy. No panic (indexing guarded).
- Fix: reject `size < 0` in `log_leaf_hashes` (or clamp callers to `>= 0`, cap at
  `log_size`).
- Prior-work: new (REPO-16 notes the O(n) recompute, not this truncation).

### R7 — `now_unix()` returns 0 on a pre-epoch clock (all timestamps zero)
- `repository/src/store.rs:2122-2127` (`now_unix`).
- `duration_since(UNIX_EPOCH).unwrap_or_default()` yields 0 if the host clock is
  pre-1970, so every `created_at`/`expires_at` becomes 0 — fresh
  challenges/sessions/pairing blobs are already-expired. Self-inflicted
  misconfiguration, not attacker-reachable.
- Fix: log a warning (or error) when `duration_since` fails rather than silently
  returning 0.
- Prior-work: new.

### R8 — `read_string_pool` pre-allocates up to ~24× the section size
- `repository/src/abi.rs:170` (`read_string_pool`).
- `Vec::with_capacity(count.min(bytes.len()))` where each element is a 24-byte
  `String` but a real entry needs ≥4 bytes, so a section of `S` bytes declaring
  `count ≥ S` forces `S*24` bytes (~24×) even though at most `S/4` strings can be
  pushed. Parsed on every `/validate` and `/publish`; ~48 MiB section → ~1.15 GiB
  transient. Same class as bug-275 (vendor loop) but a distinct site.
- Fix: bound capacity by min bytes/entry (`count.min(bytes.len()/4)`) or drop the
  pre-allocation and let the Vec grow (each push is already bounds-checked).
- Prior-work: new (audit-1 PKG covers the *compiler's* reader.rs, a different file).

### R9 — corrupted `snapshot-version` file silently disables rollback protection (fail-open)
- `repository/src/local.rs:119` (`read_snapshot_version`), consumed at
  `client.rs:741` (`verify_pinned_metadata`).
- `Ok(value.trim().parse::<i64>().ok())` — a present-but-non-numeric file yields
  `Ok(None)` and `.unwrap_or(0)` lowers the anti-rollback floor to 0 with no error,
  while the sibling `read_checkpoint` deliberately fails *closed* on the same kind
  of corruption. Robustness/consistency gap (user-owned `0o600` file).
- Fix: fail closed like `read_checkpoint` — error on a present-but-unparseable
  file.
- Prior-work: new.

### R10 — `init-root` expiry arithmetic overflows / accepts negative days
- `repository/src/main.rs:89` (`now_unix() + expires_days*24*3600`), parsed at
  `main.rs:210`.
- `--expires-days` is a bare `i64` with no range check; a huge value overflows
  (debug panic / release wrap), a negative value yields an already-expired root.
- Fix: reject non-positive `expires_days`; use `checked_mul`/`checked_add`.
- Prior-work: new (operator CLI path, outside the remote-client threat model).

## Goal

- Each item's cited fail-open / unbounded / stale / overflow edge is closed with
  the per-item fix above; no change to the happy paths or wire formats.

### Non-goals (must NOT change)

- The already-hardened decode bounds, blob path-traversal gate, TOFU pinning,
  RFC-6962 proof math, and authz choke points verified clean in this pass.
- The `.mfp`/section wire formats and endpoint shapes.

## Blast Radius

Each item is a single site (cited above). R1/R2 relate to bug-272/273 (key
lifecycle & log verification) but are distinct root causes. R8 relates to bug-275
(both are abi/payload amplification) but a distinct site.

## Fix Design

Land per item, each with its own failing test where one is cheap (R6, R8, R9, R10
are unit-testable directly; R1/R2/R3/R4/R5/R7 need small harness or injected
delay). No shared refactor required.

## Phases

### Phase 1 — tests + audit
- [ ] Add unit tests for R6/R8/R9/R10; harness/injected-delay reproductions for
      R1–R5, R7 where practical.
### Phase 2 — the fixes
- [ ] Apply each per-item fix.
### Phase 3 — validation
- [ ] Full `repository/` suite green; no wire-format/golden drift.

## Validation Plan

- Regression tests: per item as above.
- Full suite: `repository/` tests.
- Doc sync: note the response size caps (R3) and vendor/pool bounds if surfaced.

## Summary

Ten LOW, mostly-latent edges in the registry crate. The engineering is small and
localized per item; the value is closing fail-open/unbounded corners consistently
before the MVP release. No item is an active exploit today.

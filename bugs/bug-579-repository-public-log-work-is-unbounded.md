# bug-579: anonymous transparency-log endpoints recompute unbounded state

Last updated: 2026-09-11
Effort: large (3h–1d)
Severity: MEDIUM
Class: Security / denial of service

Status: Open
Regression Test: `repository/src/server.rs` route tests to be added for rate
limits and bounded proof work.

Several anonymous endpoints load every transparency-log leaf into memory and
recompute Merkle data for each request.  They have neither a route rate limit
nor a cache.  As the append-only log grows, concurrent public requests can
consume memory/CPU and serialize unrelated SQLite users behind `Store`'s one
connection mutex.

Correct behavior: public integrity endpoints remain verifiable, but request
cost is bounded and repeated requests reuse a safely invalidated checkpoint or
tree representation.

References:

- Security review, 2026-09-11 (repository-only scope)

## Failing Reproduction

Add a route test that grows the log, repeatedly calls `/log/checkpoint` and
asserts that a configured per-peer budget is enforced:

```
cargo test -p mfb_repository server::tests::log_checkpoint_is_rate_limited_and_cached
```

- Observed today: every call reaches `Store::log_leaf_hashes(None)` and rebuilds
  the tree; no limiter key is consumed.
- Expected: calls above the endpoint budget receive `429`, while normal proof
  and checkpoint verification remains correct.

## Root Cause

`repository/src/store.rs:Store::log_leaf_hashes` materializes all matching
leaves into a `Vec<[u8; 32]>`.  `repository/src/server.rs:log_checkpoint`,
`log_inclusion_proof`, `log_consistency_proof`, `package_audit`, and
`snapshot_metadata` invoke it on anonymous routes.  Unlike `/search`, these
handlers never call `RateLimiter::allow`.

## Goal

- Bound anonymous checkpoint/proof/audit work by authenticated-independent,
  per-peer limits and request-size validation.
- Avoid rebuilding an unchanged full tree on every request.

### Non-goals (must NOT change)

- Do not make transparency endpoints authenticated or alter proof wire formats.
- Do not serve a checkpoint that does not correspond to the proof response.
- Do not weaken rollback or inclusion verification in the client.

## Blast Radius

- `server.rs:log_checkpoint` — fixed in this bug.
- `server.rs:log_inclusion_proof` and `log_consistency_proof` — fixed in this bug.
- `server.rs:package_audit` and `snapshot_metadata` — fixed in this bug because
  they share full-log construction.
- `server.rs:package_detail` — separate unbounded-version response concern;
  audit during implementation, but out of scope unless it shares the selected
  cache/limiter mechanism.

## Fix Design

First establish a consistent anonymous log route budget.  Cache a signed tree
head keyed by log size, invalidating it after append; then choose either
persisted Merkle nodes or a bounded cached leaf structure for proofs.  The
cache must be coherent with the exact tree snapshot used for the response.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add red rate-limit and cache-invalidation tests.
- [ ] Census all `log_leaf_hashes` route consumers.

Acceptance: tests demonstrate currently unlimited repeated full-log work.
Commit: —

### Phase 2 — the fix

- [ ] Add route limits and a coherent checkpoint/proof cache or persisted tree.
- [ ] Apply it to every listed consumer.

Acceptance: proof verification remains valid and abusive requests are bounded.
Commit: —

### Phase 3 — full validation

- [ ] Run `cargo test -p mfb_repository`.
- [ ] Verify checkpoint, inclusion, and consistency proofs against an advancing log.

Acceptance: full suite green and response correctness is preserved.
Commit: —

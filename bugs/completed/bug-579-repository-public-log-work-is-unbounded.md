# bug-579: anonymous transparency-log endpoints recompute unbounded state

Last updated: 2026-09-12
Effort: large (3h–1d)
Severity: MEDIUM
Class: Security / denial of service

Status: **FIXED** (`460b983dd`).
Regression Test: `store::tests::signed_checkpoint_is_memoised_by_log_size`,
`server::tests::log_checkpoint_follows_an_append_and_still_verifies`,
`server::tests::anonymous_log_routes_are_rate_limited_per_ip`, and
`server::tests::the_log_budget_clears_a_full_install_burst_from_one_ip`.

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

- [x] Add red rate-limit and cache-invalidation tests.
- [x] Census all `log_leaf_hashes` route consumers.

**Both halves demonstrated at HEAD before fixing**, with a throwaway probe in a
detached worktree:

- 2000 anonymous `/log/checkpoint` requests from ONE peer all succeeded — no
  limiter key is consumed.
- Corrupting the leaf bytes **in place** (row count unchanged) was immediately
  visible in the next response. That can only happen if every request re-reads
  and re-hashes every leaf, i.e. nothing is cached.

The probe asserts the DEFECT, so it was deliberately left in the scratch
worktree and never committed.

**Census** (`grep -rn --include='*.rs' log_leaf_hashes`): five production
consumers — `log_checkpoint`, `log_inclusion_proof`, `log_consistency_proof`,
`package_audit`, `snapshot_metadata`. The report named all five. It did not
mention `package_audit_html`, which calls `package_audit` internally and does
the identical full-log work; limiting only the JSON route would have left the
HTML page as the bypass.
Commit: — (tests landed with the fix, `460b983dd`)

### Phase 2 — the fix

- [x] Add route limits and a coherent checkpoint cache.
- [x] Apply it to every listed consumer.

`Store::signed_checkpoint()` memoises `(size, root, signature)` by log size.
Sound because the log is append-only and dense — nothing in the crate deletes or
rewrites a `log_entries` row — so size uniquely determines the root.

**Two placement decisions matter more than the cache itself:**

- The memo is **not** inside `log_leaf_hashes`. That reader has a documented
  contract about surfacing a malformed leaf as an error, and
  `log_readers_reject_a_malformed_leaf_hash` corrupts leaves in place — count
  unchanged — to prove it. A size-keyed cache there would have served the
  pre-corruption leaves and silently defeated an existing test's whole point
  while leaving it green.
- `package_audit` deliberately does **not** use the memo. It needs the leaf
  vector itself to build an inclusion path per publish, so the memo would save
  nothing and would introduce a race: a head memoised at one size beside paths
  built from a differently sized leaf set. Computing both from ONE read keeps
  the response internally coherent — the property this bug's non-goals protect
  ("do not serve a checkpoint that does not correspond to the proof response").
  That route is bounded by the budget, not the cache, and the code says so.

Acceptance: **met.**
Commit: `460b983dd`

### Phase 3 — full validation

- [x] Run `cargo test -p mfb_repository`.
- [x] Verify checkpoint, inclusion, and consistency proofs against an advancing log.

`cargo test -p mfb_repository --no-fail-fast` -> **351 + 21 passed, 0 failed,
exit 0** in an isolated worktree, and **356 + 21, exit 0** re-run on the MERGED
tree after bug-585 landed in parallel — neither parent's result is evidence for
the merge.

The pre-existing round-trip tests that drive checkpoint, inclusion and
consistency against a growing log pass untouched; they were only edited to
supply the new peer extractor.

**Instrument.** The artifact gate is structurally blind to this crate (no IR, no
goldens), so its silence proves nothing. The `mfb_repository` unit suite is the
instrument.

Acceptance: **met.**
Commit: `460b983dd`

## Outcome

Fixed in `460b983dd`.

### Sizing a rate limit is a measurement, not a round number

The budget had to be derived from the **worst legitimate client**, and that
number is not small: `verify_publish_inclusion` makes three log requests
(`/log/checkpoint`, `/log/publish`, `/log/proof/:index`) and `mfb pkg install
--proof` calls it once **per dependency**. A 200-dependency install is therefore
a ~600-request burst in seconds — and behind a NAT or a CI egress address that
IP is shared by every developer on it.

That number had also just GROWN: bug-582 made every pin advance fetch a
consistency proof. A budget picked for "abuse" without measuring the client
would not have hardened the registry, it would have broken installs, and it
would have looked correct in every test written from the attacker's side.
`the_log_budget_clears_a_full_install_burst_from_one_ip` exists so that anyone
tightening the constant has to confront the client's real request shape first.

### Observing a cache needs a discriminator, and the obvious one is fake

The natural assertion — "the same bytes came back, so it was cached" — proves
nothing here, because Ed25519 signing is deterministic (RFC 8032): a full
recompute returns byte-identical output. The first version of this test asserted
exactly that and would have passed against the unfixed code.

The discriminator that does work is an in-place leaf corruption, which leaves
the memo key (the row count) unchanged: a cache hit returns the head it holds,
a recompute surfaces `malformed log leaf hash`. Generalisable: **to test a
cache, find an input the cache cannot see. If you cannot name one, the test is
not observing the cache.**

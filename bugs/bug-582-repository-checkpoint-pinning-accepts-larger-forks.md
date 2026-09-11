# bug-582: checkpoint pinning accepts an unproven larger transparency-log fork

Last updated: 2026-09-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Security / transparency-log integrity

Status: Open
Regression Test: `repository/src/client.rs` checkpoint and publish-inclusion
tests to be extended with an unproven larger fork.

Once a client has pinned checkpoint `(size=N, root=A)`, `fetch_checkpoint`
accepts any validly server-signed checkpoint with `size>N`, without requesting
or verifying a consistency proof.  It immediately overwrites the local pin.
`verify_publish_inclusion` uses this unsafe helper, so an online registry can
present a larger fork and erase the old root before any consistency check.

Correct behavior: after the first trust-on-first-use checkpoint, a candidate
larger head must not replace the pin until an RFC 6962 consistency proof proves
it extends the pinned root.

References:

- Security review, 2026-09-11 (transparency protocol pass)
- Existing contrast: `client::verify_log_consistency` already follows the
  required verify-before-pin order.

## Failing Reproduction

Add a case after the existing four-leaf checkpoint pin that serves a different,
validly signed five-leaf root but no `/log/consistency` proof:

```
cargo test -p mfb_repository client::tests::fetch_checkpoint_rejects_an_unproven_larger_fork
```

- Observed today: `fetch_checkpoint` accepts the larger checkpoint and rewrites
  `checkpoint` on disk.
- Expected: it rejects and keeps `(N, A)` unchanged.

Add the same attack to `verify_publish_inclusion`, which currently calls
`fetch_checkpoint` directly.

## Root Cause

`repository/src/client.rs:fetch_checkpoint_unpinned` rejects only a smaller
size and a different root at the *same* size.  `fetch_checkpoint` then writes a
larger candidate without consulting `/log/consistency`.  In contrast,
`verify_log_consistency` deliberately leaves the old pin intact until
`log::verify_consistency` succeeds, but no call path requires it before
`fetch_checkpoint` or `verify_publish_inclusion` advances the pin.

## Goal

- Make every checkpoint advancement consistency-proof-gated.
- Ensure failed proof retrieval/verification never changes the pin.

### Non-goals (must NOT change)

- Do not reject the first checkpoint, which has no predecessor.
- Do not change checkpoint or consistency-proof wire encodings.
- Do not rely on checkpoint signatures alone to establish append-only history.

## Blast Radius

- `client.rs:fetch_checkpoint` — fixed in this bug.
- `client.rs:verify_publish_inclusion` — fixed because it advances through the
  unsafe helper.
- `client.rs:verify_log_consistency` — unaffected semantically; use it as the
  correctness reference and avoid duplicating incompatible logic.

## Fix Design

Fold the verify-before-pin logic into the one checkpoint-advance primitive, so
all callers must obtain and verify consistency when a pin exists and the size
increases.  Preserve a separate non-pinning fetch only where a caller genuinely
needs a candidate head.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add red larger-fork tests for direct checkpoint and publish inclusion.
- [ ] Census all local checkpoint writers and candidate-head readers.

Acceptance: tests prove the larger fork currently overwrites the pin.
Commit: —

### Phase 2 — the fix

- [ ] Centralize consistency verification before every pin advance.
- [ ] Route publish inclusion through the safe primitive.

Acceptance: attacks reject without changing the old pin; valid extensions pass.
Commit: —

### Phase 3 — full validation

- [ ] Run `cargo test -p mfb_repository`.
- [ ] Re-run rollback, same-size fork, valid extension, and larger-fork cases.

Acceptance: full suite green and no unproven history can replace a pin.
Commit: —

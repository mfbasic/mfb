# bug-814: 19 of 29 `Body::Rewrite` built-ins declare fewer errors than their bodies raise

Last updated: 2026-10-04
Effort: large (3h–1d) (guess, by analogy with bug-698 Phase 2)
Severity: LOW
Class: Correctness (documentation / error contracts)

Status: Open
Regression Test: proposed — extend the `Body::Mfb` declared-errors guard
(`src/codegen/builtins/tests/mfb_body_declared_errors.rs`) to `Body::Rewrite` members.

Found while fixing bug-811 (bug-staging-8). Owner decision (bug-698, 2026-09-25): every
function must list every error it can raise.

## Failing Reproduction

No guard checks a `Body::Rewrite` member's declared errors against its body:
- `declared_errors_all.rs` covers `AbiFunction`/`AbiInline`;
- `mfb_body_declared_errors.rs` covers `Body::Mfb`;
- `general_declared_errors.rs` covers the `general` members.

A temporary patch pointed the `Body::Mfb` sweep at each `Rewrite` target's helper
block. It was reverted, see bug-811's Resolution. It measured **19 of the 29**
`Rewrite` members out of step. For example:
- `crypto::hmac` declares no errors, but its body reaches `ErrOutOfMemory`,
  `ErrInvalidArgument`, `ErrOverflow`, `ErrIndexOutOfRange` and `ErrNotFound`.
- Every `compress` codec, and most of `crypto`, lack `ErrOutOfMemory`.

## Root Cause

`Body::Rewrite` members were outside the bug-698 and bug-807 sweeps. Their
descriptors were written by hand and never checked mechanically.

## Goal

- Every `Body::Rewrite` member declares exactly the errors its body can raise,
  with per-code reachability proof as in bug-698. A code proven unreachable goes in
  the guard's `(package, member, code)` exception table.
- The guard covers `Body::Rewrite`.
- Once it lands, `mfb audit`'s hand-written fallible lists
  (`src/audit/collect/source.rs`, kept as a floor by bug-811) can be deleted.

## Blast Radius

- `crypto`, `compress` and the other `Rewrite` members: their descriptors and man prose.
- Audit output, and the man/registry goldens for those members.

## Phases

- [ ] Phase 1: add the guard for `Body::Rewrite` with a measured baseline; RED first.
- [ ] Phase 2: fill `errors` and align prose per member, with per-overload evidence.
- [ ] Phase 3: delete audit's hand lists; run the full suite and the goldens.

# plan-142-E: Prove the `Exempt` rows copy-free: `reduce`, `reduceRight`, `compress`, `crypto`

Last updated: 2026-09-20
Effort: medium (1h–2h)
Depends on: plan-142-D

Prerequisites: see plan-142-A.

Eleven self-update-shaped functions have no in-place form, because their result
is not built from `x`'s block: `collections::reduce` and `collections::reduceRight`
(the result is built from `initial`; self-update only when `U = List OF T`), six
`compress` functions (`deflate inflate gzipEncode gzipDecode zlibEncode zlibDecode`:
a new byte stream whose length is unrelated to the input's) and `crypto::argon2id`
(2 overloads) and `crypto::shake256` (a derived digest). For these the property the
plan guarantees is the one that still has meaning: **`x` is read, never copied**.
E proves that for each and records it in `SELF_UPDATE_TABLE` as `Exempt { reason, proof }`.

References: plan-142-A §3 and Open Decision 3.

## 1. Goal

- 11 rows flip `Pending("E")` → `Exempt` with a `proof` citing the lowering line that
  reads `x` without copying it.
- Each `cases.tsv` line becomes `exempt`, with a runtime check that the bytes
  allocated by `x = f(x, …)` at `|x| = 2M` exceed those at `|x| = M` by less than
  `M` element-widths plus the result's own size (i.e. no second copy of `x`).

### Non-goals

- No lowering changes unless the proof fails. If a lowering is found to copy `x`
  (e.g. an argument owned-copy), fixing that copy is in scope here (it is the plan's
  property), and the row becomes `Exempt` only after the fix.

## 2. Current State

- `reduce`/`reduceRight`: `Body::abi_inline` (`func_reduce.rs:136`,
  `func_reduce_right.rs:134`).
- `compress`: `Body::Rewrite("__compress_*")` (`builtins/compress/func_deflate.rs:87`
  etc.), a call to an MFBASIC implementation.
- `crypto`: `Body::Rewrite("__crypto_argon2id" | "__crypto_argon2idProfile" | "__crypto_shake256")`
  (`func_argon2id.rs:164`, `:181`; `func_shake256.rs:104`).
- UNVERIFIED for all 11: that the argument is passed borrowed and never copied
  (bug-665's fix may add a copy only when the callee writes a global — not the case
  here).

## Phases

### Phase 1 — Read and prove

- [ ] Add the `SelfUpdate::Exempt { reason, proof }` variant to
      `src/codegen/collection/assign/self_update.rs` (plan-142-A Correction A3: it
      was left out of A because no row constructed it), and have
      `self_update_table_has_no_stale_rows` require a non-empty `reason` and `proof`.
- [ ] For each of the 11, read the lowering (and, for `Rewrite` bodies, the called
      MFBASIC function's use of its parameter) and record the line that proves `x`
      is only read. Where a copy is found, fix it and add a regression case.
- [ ] Flip the 11 rows to `Exempt { reason, proof }`.

Acceptance: `cargo test --bin mfb self_update` → pass with 0 `Pending("E")` rows (est. 3 min).
Commit: —

### Phase 2 — Runtime proof

- [ ] Add the `exempt` byte-growth check to `rt_inplace_self_update` and flip the 11
      `cases.tsv` lines.

Acceptance: `cargo test --test rt_inplace_self_update` → pass (est. 8 min).
Commit: —

## Validation Plan

- Tests: `exempt` rows in the harness; unit census.

## Open Decisions

- If Open Decision 3 of plan-142-A resolves the other way (byte-stream builtins
  out of the census), this letter shrinks to `reduce`/`reduceRight`.

## Corrections

## Summary

Reading and measuring. Risk only if a hidden copy turns up.

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

- [x] Add the `SelfUpdate::Exempt { reason, proof }` variant to
      `src/codegen/collection/assign/self_update.rs` (plan-142-A Correction A3: it
      was left out of A because no row constructed it), and have
      `self_update_table_has_no_stale_rows` require a non-empty `reason` and `proof`.
      With the last `Pending` rows gone, `SelfUpdate::Pending` and the `pending()`
      constructor had no user left and were deleted too (dead code otherwise);
      plan-142-I Phase 1's task is updated to say so.
- [x] For each of the 11, read the lowering (and, for `Rewrite` bodies, the called
      MFBASIC function's use of its parameter) and record the line that proves `x`
      is only read. Where a copy is found, fix it and add a regression case.
      * `reduce`/`reduceRight`: `lower_collection_reduce_impl` stores `args[0]` in
        `reduce_collection` and only walks it; the result is built from `args[1]`.
        (Reading it found a separate leak — Correction E3.)
      * The six `compress` members: `Body::Rewrite` to MFBASIC helpers that touch
        `data` only through `len`, `collections::get`/`getOr` and one header-sized
        `mid` (`helper_deflate_core.rs`, `helper_inflate_core.rs`,
        `helper_gzip_frame.rs`, `helper_zlib_frame.rs`, `helper_crc32.rs`,
        `helper_adler32.rs`; `grep -nE '(=|\(|, )\s*data\b' helper_*.rs` lists every
        use). No copy.
      * `crypto::shake256`: **copied** — `__crypto_keccakSponge` began with
        `MUT msg = __crypto_copyBytes(data)`. Fixed: whole blocks are absorbed
        straight from `data`, only the final padded block is built.
      * `crypto::argon2id` (both overloads): **copied** — `__crypto_argon2H0`
        concatenated the password into its hash input, and `__crypto_blake2b`
        sliced every 128-byte block into a fresh list. Fixed: `__crypto_blake2b3`
        hashes `header ‖ password ‖ tail` without building it, and
        `__crypto_blake2bCompress` reads a block out of any list at an offset.
      The fixes landed in `04fbb45f6`; test-accept over all 1499 fixtures shows no
      runtime-output change (incl. `crypto-sha3-kat-valid`, `crypto-argon2id-valid`),
      and every changed golden was localized per function (commit message). The
      regression case is Phase 2's `exempt` check, RED-proven against the old sponge.
- [x] Flip the 11 rows to `Exempt { reason, proof }`. (10 rows: `argon2id`'s two
      overloads share one.)

Acceptance: `cargo test --bin mfb self_update` → pass with 0 `Pending("E")` rows (est. 3 min).
Verified 2026-09-21: `test result: ok. 4 passed`, no warnings; `Pending` no longer
exists.
Commit: 04fbb45f6 (the two copies), 2567f1809 (the rows)

### Phase 2 — Runtime proof

- [x] Add the `exempt` byte-growth check to `rt_inplace_self_update` and flip the 11
      `cases.tsv` lines.
      Measured on peak **live** bytes, not total bytes allocated — Correction E2 —
      with `|x|` = 65,536 and 131,072 (a `{M}` placeholder in the setup builds `x`),
      the statement's share of the peak taken as the program with it minus the same
      program without it, and a bound of half of `M` widths plus the result's own
      growth. Each line's own function is its first statement; the decoders' input is
      stored-block data built in setup, `reduce` folds with `keepAcc` (a tiny
      result), so the check is tight wherever the output allows. RED: with the old
      copying sponge, `crypto::shake256` fails — "grew the statement's peak live
      bytes by 134320 (104832 -> 239152; result length 32 -> 32), not under 33792".

Acceptance: `cargo test --test rt_inplace_self_update` → pass (est. 8 min).
Verified 2026-09-21: `test result: ok. 1 passed; 0 failed` (263.55s), 53 `arm` and
11 `exempt` lines, 0 `pending`.
Commit: 2567f1809

## Validation Plan

- Tests: `exempt` rows in the harness; unit census.

## Open Decisions

- If Open Decision 3 of plan-142-A resolves the other way (byte-stream builtins
  out of the census), this letter shrinks to `reduce`/`reduceRight`.

## Corrections

- **E1 (Phase 1): two real copies, both in `crypto`.** §2 marked all 11 "UNVERIFIED"
  and expected the argument to be passed borrowed; it is, but `shake256`'s sponge
  and `argon2id`'s H0/BLAKE2b copied it internally. Both fixed (Phase 1).
- **E2 (Phase 2): the check measures live bytes.** As written — total bytes
  allocated at `2M` vs `M` — it failed 8 of 11 lines with no copy of `x` present:
  `shake256` grew 3,124,720 B for 8,192 more input bytes (381 B per byte, with a
  32-byte result), `argon2id` ~140 B/byte, `deflate` ~17 B/byte — each function's
  per-block temporaries (Keccak/BLAKE2b rounds, deflate's tables), allocated and
  freed as it goes. Total allocation cannot tell those from a copy; peak live bytes
  can, because a copy of `x` is live alongside `x` until the assignment. Measured:
  the statement's peak share grew +9712 with and without `shake256` alike (no
  copy), +19488 extra for `deflate` at `M` = 8,192 (its `min(n, 32768)`-entry match
  table), so `M` is 65,536, above that cap. The bound is *half* of `M` widths — a
  copy of exactly `M` bytes must fail even with noise — plus the result's growth:
  stricter than the plan's `M` widths, not looser.
- **E3 (Phase 2): a leak in `reduce`, fixed first.** Measuring `reduce` showed its
  peak live bytes growing 44 B per element: a `List`/`Map`/`Set` accumulator leaked
  every superseded block (plan-86-B only reclaimed `String` accumulators). Fixed in
  its own commit (`a9618a095`, `tests/runtime/rt_reduce_collection_accumulator_frees.rs`,
  RED on six shapes before).

## Summary

Reading and measuring. Risk only if a hidden copy turns up.

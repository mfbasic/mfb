# bug-11: A Fixed literal with a large exponent makes `expand_scientific_notation` build a multi-GB string (compile-time hang/OOM) and can overflow i32

Last updated: 2026-07-08
Effort: small (<1h)

`src/numeric.rs::expand_scientific_notation` expands a scientific-notation
decimal string into a plain decimal by materializing **every** shifted-in zero as
a literal character. It runs as the *first* step of `fixed_raw_from_decimal`
(`numeric.rs:94`), **before** any of that function's `checked_mul`/`checked_add`
range guards, and it has no bound on the exponent magnitude. A source file
containing a Fixed literal such as `1e-1000000000F` therefore drives an ~10⁹-byte
`String` allocation and fill loop — a multi-second hang and ~1 GB memory spike at
compile time — with no diagnostic. At the i32 boundary the point computation
itself overflows.

The single correct behavior a fix produces: a Fixed literal whose exponent is
out of the representable Fixed range is **rejected with a clean "out of range"
diagnostic in O(1)**, never expanded digit-by-digit, and the point arithmetic
never overflows.

Because building third-party MFBASIC *source* is a normal operation (packages
ship source), this is a reachable compile-time DoS. Severity MEDIUM: it is a
denial-of-service / debug-build panic, not memory corruption.

References:

- `src/numeric.rs:46-83` (`expand_scientific_notation`) — `:61`
  (`point = int_part.len() as i32 + exponent`, unchecked i32 add), `:66-76`
  (the `0..(-point)` and `0..(point - digits.len())` zero-fill loops).
- `src/numeric.rs:91-144` (`fixed_raw_from_decimal`) — `:94` calls
  `expand_scientific_notation` before the `checked_*` guards at `:118-141`.
- Reached from `native_immediate_value` and the `ir::lower` fold (bug-07).
- Lexer accepts unbounded exponent digit runs (`src/lexer.rs`, `scan_base_digits`
  — no magnitude cap).
- Found during goal-01 review of `src/numeric.rs`.

## Failing Reproduction

```
# any .mfb the compiler builds:
LET x AS Fixed = 1e-1000000000F
```

- Observed: compile hangs for seconds and spikes to ~1 GB RSS while
  `expand_scientific_notation` pushes ~10⁹ `'0'` characters, then
  `fixed_raw_from_decimal` finally rejects the giant `whole` string. With
  `1e2147483647F` on an overflow-checked (debug) build, `int_part.len() as i32 +
  exponent` panics with "attempt to add with overflow".
- Expected: an immediate `Fixed constant … is out of range` diagnostic, O(1), no
  large allocation and no overflow.

Contrast cases that are correct today:

- `1e9999999999F` — the exponent does **not** fit i32, so
  `exponent_text.parse::<i32>()` fails, `expand_scientific_notation` returns the
  string unchanged, and `whole.parse::<i128>()` rejects it cleanly. Safe.
- Ordinary literals (`2.5e2`, `1e-3`, `3.14F`) expand to a handful of digits.
- So the dangerous window is exponents that **fit i32 but are large in
  magnitude**: roughly `|e| ≳ 10⁶` for a practical hang, `|e| ≈ i32::MAX` for the
  overflow panic.

## Root Cause

`expand_scientific_notation` (`numeric.rs:46`) is written for well-formed small
literals and materializes the fully-expanded decimal string. Two unbounded
constructs: (1) `point = int_part.len() as i32 + exponent` (`:61`) is a plain i32
add that overflows when `exponent` is near `i32::MAX`; (2) the zero-fill loops
(`:68`, `:74`) iterate `|point|` times, each pushing one byte, so the output —
and the work — is proportional to the exponent magnitude. Because this expansion
happens *before* `fixed_raw_from_decimal`'s `checked_*` range guards, those guards
never get the chance to reject the value cheaply; the blow-up is already underway.

## Goal

- A Fixed literal whose (post-shift) magnitude cannot fit the 32.32 Fixed range is
  rejected in O(1) with the existing out-of-range diagnostic.
- `expand_scientific_notation` computes the point position without integer
  overflow and never allocates an output larger than a small bounded budget.

### Non-goals (must NOT change)

- Do not change the exact-digit expansion result for in-range literals (goldens
  for `2.5e2` etc. must be byte-identical).
- Do not silently clamp a huge exponent to a finite value — reject it.

## Blast Radius

- `fixed_raw_from_decimal` (`numeric.rs:91`) — the Fixed-constant path; fixed by
  this bug.
- `expand_scientific_notation`'s other caller, the plan-28-B `toString` fold
  (folds a scientific-notation literal's `toString` to plain decimal) — same
  unbounded expansion; must get the same cap.
- Float literals do not route through here (they parse via `f64`), so they are
  unaffected.

## Fix Design

Before the zero-fill, bound the exponent/point to the representable Fixed range.
Fixed is 32.32, so the integer part needs at most ~10 decimal digits and the
fraction ~10; any `point` outside roughly `[-40, 40]` (a generous budget) cannot
produce an in-range Fixed value and should early-return
`Err("Fixed constant … is out of range")` from `fixed_raw_from_decimal` (or have
`expand_scientific_notation` signal the caller). Compute `point` in `i64` (or with
`checked_add`) so `int_part.len() as i64 + exponent as i64` never overflows.

Rejected alternative: capping only the *allocation* size but still looping —
still wastes work proportional to the cap; a magnitude check on `point` is O(1)
and strictly better.

## Phases

### Phase 1 — failing test + audit

- [ ] Add a `numeric.rs` unit test: `fixed_raw_from_decimal("1e-1000000000")`
      returns an out-of-range `Err` quickly (assert it does not allocate a huge
      string — e.g. wrap with a small time/size bound or assert on the error).
- [ ] Add a test for the i32-overflow input (`1e2147483647`) proving no panic.
- [x] Blast-radius audit complete (above).

Acceptance: tests fail today (hang / debug panic).
Commit: —

### Phase 2 — the fix

- [ ] Compute `point` in i64/checked; reject `|point|` beyond the Fixed budget
      before any zero-fill; apply the same guard to the toString-fold caller.

Acceptance: Phase 1 tests pass; in-range literals unchanged.
Commit: —

### Phase 3 — validation

- [ ] `scripts/test-accept.sh` — in-range Fixed goldens byte-identical; add
      accept fixtures for the new rejection diagnostic if the project convention
      requires them.

Acceptance: full suite green; only the new rejection path is added.
Commit: —

## Validation Plan

- Regression test(s): the `numeric.rs` unit tests above.
- Runtime proof: build a `.mfb` with `1e-1000000000F` and observe the immediate
  out-of-range diagnostic instead of a hang.
- Doc sync: none expected (out-of-range Fixed literals are already an error class).
- Full suite: `scripts/test-accept.sh`.

## Summary

The risk is choosing the right rejection budget so no in-range literal regresses;
the fix is a magnitude guard plus i64 point arithmetic in one small function,
shared with the toString-fold caller.

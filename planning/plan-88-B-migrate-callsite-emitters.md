# plan-88-B: migrate the per-call-site emitters to raise_error

Last updated: 2026-08-03
Effort: large (3h–1d)
Depends on: plan-88-A (the `raise_error` / `raise_error_bare` primitives, the
used-set, and the Phase-0 parity audit must be landed — if A is not complete,
B cannot start, full stop.)

Sub-plan **B** of plan-88. See plan-88-A §3 for the overall design and the
**per-site acceptance gate** (rt-error test green before and after; code change =
fail; message change only per the Phase-0 consolidation list). B migrates every
**per-call-site** error emitter — the 12 `emit_*_return` wrapper calls and the
direct `emit_error_code_return` allocation calls — to `raise_error(func_id, name)`
or `raise_error_bare(name)`, one file at a time, each gated on that per-site test.
It leaves the symbol-path sites (C) and all deletions (D) untouched.

Behavioral outcome for B: **every per-call-site error emission goes through
`raise_error`/`raise_error_bare`, and every migrated site’s rt-error test raises
the same `Error.code` after as before.**

References: plan-88-A; `src/target/shared/code/builder_error_emission.rs`;
the caller files enumerated below; `.ai/compiler.md`; `scripts/artifact-gate.sh`.

## Prerequisites

See plan-88-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-88-A complete | `ls planning/completed/plan-88-A-*` (exists) | NOT MET (A pending) |
| `raise_error` / `raise_error_bare` exist | `grep -n 'fn raise_error' src/target/shared/code/builder_error_emission.rs` | NOT MET until A |

## 1. Goal

- Zero remaining `self.emit_*_return()` wrapper calls and zero direct
  `self.emit_error_code_return(ERR_*_CODE, ERR_*_MESSAGE)` allocation calls
  outside the wrapper bodies themselves; every one replaced by
  `raise_error`/`raise_error_bare`. Wrapper *definitions* remain (deleted in D).

### Non-goals

- Do **not** migrate the symbol path (`push_error_message_address`) — that is C.
- Do **not** delete any wrapper, `ERR_*` constant, or gating — that is D.
- **No runtime behavior change:** every migrated site raises the same `Error.code`
  (and the same message, save the Phase-0 consolidation cases). Codegen bytes are
  not a constraint here.

## 2. Current State (delta from A)

After A, `raise_error`/`raise_error_bare` exist and delegate to
`emit_error_code_return`; the `collections.get` poc already uses `raise_error`.
Remaining per-call-site emitters (measured, plan-88-A §2):

| Emitter | Calls | Bucket |
|---|---|---|
| `emit_allocation_error_return` | 61 | func or bare per site |
| `emit_invalid_argument_return` | 26 | func or bare per site |
| `emit_overflow_return` | 21 | mostly bare (operator) |
| `emit_index_out_of_range_return` | 9 (8 after poc) | func (collection/string builtins) |
| `emit_invalid_format_return` | 7 | func or bare |
| `emit_float_domain_return` | 6 | bare (operator) / math-builtin func |
| `emit_not_found_return` | 5 | func |
| `emit_float_nan_return` | 3 | bare |
| `emit_float_inf_return` | 3 | bare |
| `emit_underflow_return` | 2 | bare |
| `emit_float_overflow_return` | 2 | bare |
| `emit_encoding_error_return` | 1 | func |
| direct `emit_error_code_return` (allocation) | 25 | func or bare per site |

Command for the live counts: `grep -rhoE 'self\.emit_[a-z_]+_return\(\)'
src/target/shared/code/*.rs | sort | uniq -c | sort -rn` and the direct-call grep
in plan-88-A §2.

### Verified properties

- **Func-vs-bare is a per-site property**, decided by the enclosing function: if
  the emit is inside a named-builtin lowering (e.g. `lower_list_get_common` =
  `collections.get`, `builder_strings_builtins.rs` = `strings.*`), it is
  `raise_error(func_id, name)`; if inside an operator/observation-boundary path
  (integer overflow, float boundary in `builder_*_math.rs`, `float_format.rs`) or
  TRAP, it is `raise_error_bare(name)`. This classification is **per site** and is
  the actual work of B — it is not derivable from the wrapper name alone (e.g.
  `emit_overflow_return` is called from both `toInt`/`toByte` conversion builtins
  *and* raw integer operators).

## 3. Design

Migrate **one caller file at a time**, smallest first, gate each on the per-site
rt-error test (plan-88-A). For each site: (a) identify the enclosing function; (b)
if it maps to a `BuiltinFunction`, add the raised error name to that function’s
`errors` array in `src/builtins/*.rs` (the contract the `raise_error`
`debug_assert` checks) and call `raise_error("pkg.fn", "ErrX")`; (c) else call
`raise_error_bare("ErrX")`. The `errors` additions are line-neutral where
possible to avoid rippling importer `.ir` goldens (per the “builtin .mfb source
ripples to importer IR goldens” hazard — but these `errors` are Rust descriptor
data, not `.mfb` source, so `.ir` goldens are unaffected; confirm with a spot
`git diff --stat tests/**/*.ir` after the first descriptor edit).

Risk concentrates in the two high-volume families (`allocation` 61+25,
`invalid_argument` 26): they span the most files and both buckets. Do them last,
after the pattern is proven on the small operator families.

## Phases

Order: operator families (bare, self-contained) → func families → the two
high-volume families.

### Phase 1 — operator/float bare families

The float + over/underflow families are (mostly) operator sites → `raise_error_bare`.

- [ ] Migrate all `emit_float_domain_return` / `float_nan` / `float_inf` /
      `float_overflow` / `underflow` / `overflow` call sites (37 calls, plan-88-A
      §2) to `raise_error_bare("ErrFloat*"/"ErrOverflow"/"ErrUnderflow")`, EXCEPT
      any site inside a named math/conversion builtin — those get
      `raise_error(func_id, name)` and an `errors` entry on that builtin. Classify
      each by its enclosing `fn`.
- [ ] Add `errors` entries to any math/conversion builtin that raises here
      (`src/builtins/math.rs`, `general.rs`) — e.g. `math.sqrt` → `["ErrFloatDomain"]`.
- [ ] Tests: for each family, ensure a `tests/rt-error/**` fixture triggers it and
      asserts `Error.code` — passing **before** migration (add if missing; several
      float/overflow fixtures already exist, e.g. `rt_native_size_arith_overflow`,
      `tests/rt-error/math/func_math_sqrt_fixedarray_rt`). Re-run **after**.

Acceptance (per the per-site gate): no `emit_float_*_return`/`emit_overflow_return`/
`emit_underflow_return` call sites remain (`grep -rc` → 0 each); every relevant
`tests/rt-error/**` test raises the same `Error.code` after as before; message
changes, if any, are on the Phase-0 consolidation list. `cargo test --bin mfb`
green.
Commit: —

### Phase 2 — func families (index/not_found/encoding/invalid_format)

Single-bucket func families, one file cluster each.

- [ ] Migrate the 8 `emit_index_out_of_range_return` sites (`list_mutate.rs`,
      `builder_search.rs`, `builder_strings_builtins.rs`) to `raise_error(func_id,
      "ErrIndexOutOfRange")`, adding `"ErrIndexOutOfRange"` to each owning builtin’s
      `errors` (`collections.set/insert/removeAt`, `strings.mid`, …).
- [ ] Migrate `emit_not_found_return` (5), `emit_encoding_error_return` (1),
      `emit_invalid_format_return` (7) likewise, classifying each site’s owner and
      declaring the error.
- [ ] Tests: an `tests/rt-error/**` fixture per family (index-out-of-range,
      not-found, encoding, invalid-format) triggering it and asserting `Error.code`;
      green before and after each migration.

Acceptance (per-site gate): those four wrapper call-counts are 0; the rt-error
fixtures raise the same `Error.code` after as before; `cargo test --bin mfb` green.
Commit: —

### Phase 3 — high-volume families (allocation, invalid_argument)

Largest blast radius; do last, pattern already proven.

- [ ] Migrate all 61 `emit_allocation_error_return` + 25 direct
      `emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)`
      sites. Allocation failure inside a builtin lowering → `raise_error(func_id,
      "ErrOutOfMemory")` + declare; inside a shared allocator/arena helper with no
      owner → `raise_error_bare("ErrOutOfMemory")`. Classify each.
- [ ] Migrate all 26 `emit_invalid_argument_return` sites likewise.
- [ ] Add `errors` entries for every builtin that raises `ErrOutOfMemory` /
      `ErrInvalidArgument` (expect a broad set across `collections`/`strings`/
      `crypto`/`money`/etc.).
- [ ] Tests: `tests/rt-error/**` fixtures triggering an allocation-failure path
      and an invalid-argument path, asserting `Error.code`; green before and after.

Acceptance (per-site gate): `grep -rc 'self\.emit_allocation_error_return\|self\.emit_invalid_argument_return'
src/target/shared/code/*.rs` sums to 0, and `emit_error_code_return` has no
`ERR_*`-constant caller left outside `builder_error_emission.rs`; the rt-error
fixtures raise the same `Error.code` after as before; `cargo test --bin mfb` green.
Commit: —

## Validation Plan

- Tests: `cargo test --bin mfb` after each phase; the `raise_error` `debug_assert`
  is the per-site contract check (fires in the debug test build if a site raises
  an undeclared error).
- Coverage check: the migrated sites are exercised by existing collection/string/
  math runtime tests — confirm those are in the bin suite (not filtered out).
- Runtime proof: a `.mfb` triggering an allocation failure path and an
  out-of-range path still raise the same codes/messages (e.g. compare `Error.code`
  and message before/after B on a scripted program).
- Codegen goldens: run `scripts/artifact-gate.sh` once at B’s close only as a
  sanity sweep — since `raise_error` and the `emit_*_return` wrapper bottom out in
  the same `emit_error_code_return`, a golden that *does* move points at a mistake
  (wrong error name / wrong bucket) to investigate, not to re-baseline blindly.
  This is a diagnostic, not the gate. Do not run concurrently with another gate
  (`pgrep -f artifact-gate` first); once (~15-20 min).
- Doc sync: none (messages unchanged in B).
- Acceptance: the per-site rt-error tests (green before and after each migration,
  same `Error.code`) + `cargo test --bin mfb` green.

## Open Decisions

- None new; B inherits A’s Open Decisions (physical symbol scheme, resolved in A).

## Corrections

- **Incremental wrapper deletion (deviation, pulls part of D forward).** AGENTS.md
  forbids leaving dead code, so a `emit_*_return` wrapper is deleted the moment its
  last caller migrates — not batched in D. `emit_underflow_return` was deleted in
  the first B increment (both callers were in `builder_numeric.rs`). D still owns
  the `ERR_*` **const** deletions (the wrappers' bodies reference them until then).
- **Func-vs-bare classification finding (Phase 1).** The operator/float family is a
  genuine mix, resolved by reading each enclosing fn:
  - **Bare** (shared helpers / observation boundaries, no single owner):
    `builder_numeric.rs` (all — `emit_integer_binary_checked`, `emit_fixed_multiply/
    divide`, `emit_float_binary/pow`, checked multiply/division); `builder_math.rs`
    `emit_float_result_check`/`_fp` (the float NaN/Inf/Overflow observation boundary,
    shared by every float op) and `emit_float_exponent_classify` (:876).
  - **Func** (1:1 with a builtin's codegen → `raise_error(func, name)` + declare on
    the builtin in `src/builtins/*.rs`): `builder_math.rs:1088` = `math.sqrt`
    (`ErrFloatDomain`); `:457` = `math.abs` (`ErrOverflow`); `builder_conversions.rs`
    `lower_to_byte` = `toByte`, `lower_to_float` = `toFloat` (`ErrOverflow`), etc.
  Each func site needs its error added to that builtin's `errors` array BEFORE the
  site is converted, or `raise_error`'s `debug_assert` fires.

### Resume state (plan-88-B Phase 1, IN PROGRESS)

- **Done + committed:** `builder_numeric.rs` (13 bare sites → `raise_error_bare`),
  dead `emit_underflow_return` deleted. Commits `f5e3bc90b` (+ plan-88-A complete:
  `978cbc16f`, `5a58a8ea7`).
- **Remaining Phase 1 sites (24):** `builder_conversions.rs` (7 — classify
  `lower_to_*` func vs shared), `builder_fixed_math.rs` (2, bare), `builder_money_math.rs`
  (1, bare/money-func), `builder_math.rs` (5 — `math.abs`/`math.sqrt` func,
  result-check bare), `builder_simd_math.rs` (2), `builder_simd_float_math.rs` (3).
  Then declare `errors` on `math.sqrt`/`math.abs`/`toByte`/`toFloat`, and delete the
  now-dead `emit_overflow`/`float_domain`/`nan`/`inf`/`float_overflow` wrappers once
  their last callers migrate.
- **Gate:** existing `tests/rt-error/{arithmetic,math,money,...}` fixtures cover
  these; each must raise the same `Error.code` before and after (bare sites are
  byte-identical; func sites too — same code+message, plus a declared contract).

## Summary

B is the bulk mechanical migration of the 172 per-call-site emitters, ordered
operator-bare → func → high-volume, each file gated on the per-site rt-error test
(same `Error.code` before and after). The real work is per-site func-vs-bare
classification and the matching `errors` declarations, not the call rewrite. The
symbol path and all deletions stay for C and D. Untouched: the error a program
raises (unchanged), `ErrWrapped`, and the wrapper/`ERR_*` definitions.

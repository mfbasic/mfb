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

- [x] Migrated all 37 call sites. **Bare** (shared helpers): `builder_numeric.rs`
      (13), `builder_fixed_math`/`money_math`/`simd_math`/`simd_float_math` (7),
      `builder_conversions.rs` shared helpers `emit_float_to_int_value`/
      `emit_int_parse_sign_epilogue`/`emit_integer_to_fixed_value`/
      `emit_float_bits_to_fixed_value` (4), `builder_math.rs` `emit_float_result_check`/
      `_fp` + `emit_float_exponent_classify` (7). **Func**: `builder_conversions.rs`
      `lower_to_byte`→`toByte`, `lower_to_float`→`toFloat`, `lower_to_fixed`→`toFixed`
      (`ErrOverflow`); `builder_math.rs` `math.abs`(`ErrOverflow`)/`math.sqrt`
      (`ErrFloatDomain`).
- [x] Declared errors on the func builtins via new `gfn_err`/`mf_err` const helpers:
      `toByte`/`toFloat`/`toFixed` → `["ErrOverflow"]` (`general.rs`); `math.abs` →
      `["ErrOverflow"]`, `math.sqrt` → `["ErrFloatDomain"]` (`math.rs`).
- [x] Tests: `cargo test --bin mfb` 3753 green; `tests/rt-error/general/toByte_overflow`
      (→ `7-705-0010`) and `tests/rt-error/math/func_math_sqrt_float_domain_rt`
      (→ `7-705-0012`) built + run, byte-identical to their goldens (the func-site
      `debug_assert` passed, proving the declarations are correct). Deleted the 6
      now-dead operator/float wrappers; fixed 3 dangling man citations
      (`toFixed`/`toMoney`/`addMonths`) that referenced `emit_overflow_return`.

Acceptance (per-site gate) MET: `grep -rcE 'self\.emit_(overflow|underflow|float_domain|
float_nan|float_inf|float_overflow)_return\(\)' src/target/shared/code/*.rs` → 0;
rt-error fixtures raise the same `Error.code` after as before; `cargo test` green.
Commit: f5e3bc90b, a52a2e8a4, 0e3254f5a

### Phase 2 — func families (index/not_found/encoding/invalid_format)

Single-bucket func families, one file cluster each.

- [x] Migrated all `emit_index_out_of_range_return` sites (9): `list_mutate.rs`
      `collections.insert`/`set`/`removeAt`; `builder_search.rs` `strings.find`/
      `strings.mid` + `collections.find`/`collections.mid`; `builder_strings_builtins.rs`
      `strings.graphemeAt`. Declared `ErrIndexOutOfRange` on each (collections via
      `native` errors; strings via new `strings_fn_err`).
- [x] Migrated `emit_not_found_return` (5): `builder_collection_query.rs`
      `lower_map_get` → `collections.get` (`ErrNotFound`, added to its errors);
      `builder_search.rs` find sites → `strings.find`/`collections.find`.
      `emit_encoding_error_return` (1): `emit_byte_list_to_string_value` (shared) →
      `raise_error_bare("ErrEncoding")`. `emit_invalid_format_return` (7):
      `lower_to_float`/`lower_to_fixed`/`lower_to_money` func (`toFloat`/`toFixed`/
      `toMoney` declare `ErrInvalidFormat`); the `emit_*`/money helpers bare.
- [x] Tests: `cargo test` 3753 green; `func_collection_set_out_of_range` (7-705-0001),
      `func_collection_get_not_found` (7-705-0004), `func_collection_find_out_of_range`,
      `toFixed_invalid_format` built + run byte-identical to goldens (func
      `debug_assert`s pass). Deleted the 4 dead wrappers; fixed 2 more man citations
      (`toFixed`/`toMoney` invalid-format).

Acceptance (per-site gate) MET: `grep -rcE 'self\.emit_(index_out_of_range|not_found|
encoding_error|invalid_format)_return\(\)' src/target/shared/code/*.rs` → 0; rt-error
fixtures byte-identical; `cargo test` green.
Commit: d9316c0ea

### Phase 3 — high-volume families (allocation, invalid_argument)

Largest blast radius; do last, pattern already proven.

- [x] Migrated all 86 `ErrOutOfMemory` sites (61 `emit_allocation_error_return` +
      25 direct). **All bare** — corrected from the plan's per-builtin idea:
      `ErrOutOfMemory` is a system-level allocator error with no single owning
      builtin (raised at shared arena-alloc points). The 25 direct sites are
      byte-identical; the 61 `emit_allocation_error_return` sites are
      runtime-equivalent (they used the x0-register optimization — `_mfb_arena_alloc`
      returns the OOM code in x0, per the bug-352 guard — so `raise_error_bare`'s
      immediate 77010001 is the same error, but `.ncode` bytes change).
- [x] Migrated all 26 `emit_invalid_argument_return` sites (func: strings.split/
      count/repeat, toScalar declare `ErrInvalidArgument`; bare: dispatchers/shared
      helpers). Byte-identical.
- [x] Declared errors on the func builtins (no `ErrOutOfMemory` declarations — it is
      bare). `emit_error_code_return` has no `ERR_*`-constant caller left; deleted the
      `emit_allocation_error_return` + `emit_invalid_argument_return` wrappers.
- [x] Tests: `cargo test` 3753 green; invalid_argument byte-identical. Allocation
      untestable via rt-error (can't force OOM) — verified runtime-equivalent by
      construction (source diff is ONLY `raise_error_bare("ErrOutOfMemory")` swaps;
      OOM code+message unchanged). All 112 byte-identity `.ncodesum` goldens
      re-baselined via `scripts/regen-ncodesum.sh`; collections + strings gates
      re-verified **0 diffs**.

Acceptance (per-site gate) MET: `grep -rc 'self\.emit_allocation_error_return\|self\.emit_invalid_argument_return'
src/target/shared/code/*.rs` → 0; `cargo test --bin mfb` green; the 112 byte-identity
`.ncodesum` goldens refreshed to the unified allocation codegen.
Commit: 8a336ea95 (code) + 402aa0596 (goldens)

> bug-352 guard test (`no_overflow_label_returns_through_the_result_tag_register`,
> `tests.rs`) is now moot — it scanned for `emit_allocation_error_return` after
> overflow labels; the symbol is deleted and `raise_error_bare` never reads x0, so
> the footgun is structurally gone. Left in place (passes trivially, offenders empty);
> D may remove it.

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

- **New `errors`-declaring const helpers (deviation from threading through the base
  helper).** Rather than add an `errors` param to every `gfn`/`mf` call site (18 and
  ~40 respectively), added `gfn_err`/`mf_err` variants that reuse the base helper and
  set `.errors` (const-fn local mutation). Only the ~5 func builtins change call
  sites. `collections.native` already threads `errors` directly; either shape is
  fine — the field is what matters.
- **Man-citation maintenance.** Deleting `emit_overflow_return` dangled 3 man-page
  citations (`general/toFixed`, `general/toMoney`, `datetime/addMonths`) that
  referenced it as the overflow-emission provenance. Repointed to `raise_error`
  (toFixed, a func site in `builder_conversions.rs`) / `raise_error_bare` (the shared
  primitive). `man_citations_resolve` green.

### Resume state (plan-88-B Phase 1 COMPLETE; Phase 2 next)

- **plan-88-A complete:** `978cbc16f`, `5a58a8ea7`.
- **plan-88-B Phase 1 complete** (37 sites): `f5e3bc90b` (builder_numeric),
  `a52a2e8a4` (fixed/money/simd), + this commit (conversions/math func+bare, wrapper
  deletions, `gfn_err`/`mf_err`, man citations).
- **Next — Phase 2** (func families): `emit_index_out_of_range_return` (8),
  `emit_not_found_return` (5), `emit_encoding_error_return` (1),
  `emit_invalid_format_return` (7). Then Phase 3 (allocation 61+25, invalid_argument
  26). Same pattern: classify, declare on the owning builtin, convert, delete the
  wrapper when its last caller migrates.

## Summary

B is the bulk mechanical migration of the 172 per-call-site emitters, ordered
operator-bare → func → high-volume, each file gated on the per-site rt-error test
(same `Error.code` before and after). The real work is per-site func-vs-bare
classification and the matching `errors` declarations, not the call rewrite. The
symbol path and all deletions stay for C and D. Untouched: the error a program
raises (unchanged), `ErrWrapped`, and the wrapper/`ERR_*` definitions.

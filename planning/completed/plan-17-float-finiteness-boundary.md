# MFBASIC Float Finiteness at Observation Boundaries Plan

Last updated: 2026-06-28

> **Status: IMPLEMENTED (2026-06-28).** Pieces A and B landed in full. Piece C
> was narrowed per the user's direction: `ErrFloatDomain` (`77050012`) is
> **kept** — only the float `/` divide-by-zero pre-check was removed (its `±Inf`
> /`NaN` now traps at the boundary as `ErrFloatOverflow`/`ErrFloatNaN`). The
> other `ErrFloatDomain` users stay: `Float MOD 0` (the exact `fmod` kernel needs
> a non-zero divisor), the `^` operator's whole/non-negative exponent guard, and
> the `math::` domain checks (`sqrt`/`log`/`log10`/`asin`/`acos`). At an
> observation boundary an escaping `±Inf` raises `ErrFloatOverflow` (matching the
> spec's "arithmetic overflow to infinity"); the per-op finiteness check is
> emitted only when the boundary value is a fresh arithmetic node (`Binary`/
> `Unary`), so reads/constants/call-results — finite by the invariant — cost
> nothing and finite-float output is byte-identical. Boundary choke point:
> `CodeBuilder::observe_float` (`builder_math.rs`); IEEE `<`/`<=` use the new
> `b.mi`/`b.ls` conditions.

MFBASIC guarantees that a program can never observe a non-finite `Float`. Today
that guarantee is enforced the strictest possible way — a finiteness check after
**every** float-producing operation — which costs ~7 instructions per op and
dominates the float benchmark loops (mandelbrot 4.5×, nbody 8.2×, leibniz 2.6× vs
`c -O2`; the check is most of that gap).

This plan moves the guarantee from *per-operation* to *per-observation-boundary*.
The invariant becomes its true intent: **no value of `NaN`/`Inf` is ever
user-accessible.** Intermediate, anonymous expression temporaries may be non-finite
transiently; the check fires only where a `Float` becomes observable (a named
binding, a collection/record store, a return, an argument, a print/convert). A
transient that recovers to finite — e.g. `1.0 / (1e200 * 1e200)` → `+0.0` — is
*correct* and does not trap.

The single behavioral outcome a correct implementation produces: every `Float` a
program can read, store durably, print, return, or pass is finite; everything else
is faster because it is no longer individually checked.

It complements:

- `./mfb spec language types` (`src/spec/language/04_types.md` §3 — the
  finiteness guarantee this restates; canonical specs under `src/spec/**`)
- `./mfb spec diagnostics error-codes` (`src/spec/diagnostics/02_error-codes.md` —
  `ErrFloatDomain` `77050012` is **removed**; this table is the `errorCode::` build
  input)

## 1. Goal

- Relax the float finiteness rule to **"no user-accessible `Float` is non-finite."**
  A finiteness check is emitted only when a `Float` value crosses an **observation
  boundary**, not after each arithmetic op.
- A non-finite **intermediate** is permitted and may recover to finite without
  trapping. When a non-finite *does* reach a boundary, it traps there — at that
  statement's `line:char`, with the error reflecting the value's class
  (`ErrFloatNaN` / `ErrFloatInf` / `ErrFloatOverflow`).
- **Float comparisons follow IEEE 754** for non-finite operands (no trap):
  any comparison involving `NaN` is `false`; `+Inf > x` true / `+Inf < x` false;
  `-Inf > x` false / `-Inf < x` true.
- **Remove `ErrFloatDomain` (`77050012`).** Division by zero and invalid `^`
  produce `Inf`/`NaN` like any other op and are caught at the boundary as
  `ErrFloatInf` / `ErrFloatNaN`. The explicit divide-by-zero pre-check is removed.

### Non-goals (explicit constraints)

- **No change for finite floats.** Every program whose floats stay finite produces
  byte-identical output. Only programs that today trap on a non-finite *and* whose
  value recovers/escapes differently are affected.
- **The user-facing invariant is not weakened** — a non-finite must never be
  readable from a named variable, collection element, record field, function
  result/argument, or printed/converted output. "Relax per-op" must not become
  "leak a non-finite."
- **No change to `Integer`/`Byte`/`Fixed`** semantics, or to float value layout
  (`mfb spec memory`), copy/transfer, or map-key comparison (keys stay **bitwise**:
  `+0.0`≠`-0.0`, `NaN`=`NaN` — distinct from the IEEE *value* comparison above).

## 2. Current State

- `emit_float_result_check_fp` (src/target/shared/code/builder_math.rs:1027) is the
  FP-domain finiteness check (`fabs`/`fcmp` vs `+Inf`, 3-way finite/inf/nan). It is
  called **per op** from `emit_float_binary` (builder_numeric.rs:858, for
  `+ - * / MOD ^`) and the pow/math paths; `emit_float_result_check` (the GP-bits
  variant, builder_math.rs:981) is used where the result is already a GPR.
- Provably-finite ops already skip the check: integer→float (`scvtf`) and negation
  (`lower_numeric_unary_negation`). This plan extends the "skip" to *all* anonymous
  intermediates.
- Division emits a domain pre-check (`float_compare_zero_d` → `emit_float_domain_return`
  → `ErrFloatDomain`) before the divide; pow emits domain returns too.
- `ErrFloatDomain` `77050012` lives in `src/spec/diagnostics/02_error-codes.md` and
  `error_constants.rs` (`ERR_FLOAT_DOMAIN_*`), surfaced via `errorCode::`.
- Float comparisons lower through `lower_comparison_binary` (uses `fcmp`); they have
  never seen a non-finite because none could be constructed.

## 3. Design Overview

Three pieces, lowest-risk first; the check-move is the core and lands behind the
spec + comparison groundwork.

- **Piece A — observation-boundary check (the core).** Stop checking in
  `emit_float_binary`/pow; instead emit the finiteness check via a single choke
  point `observe_float(value)` wherever a `Float` becomes user-accessible. The
  correctness risk is the **boundary audit**: every observation site must call it,
  or a non-finite leaks.
- **Piece B — IEEE comparisons.** Define and verify float `< > <= >= = <>` for
  non-finite operands via `fcmp` + IEEE condition codes (likely already correct;
  the work is removing any finite-only assumption and proving the truth table).
- **Piece C — drop `ErrFloatDomain`.** Remove the divide-by-zero pre-check and the
  pow domain returns; let `Inf`/`NaN` flow to the boundary. Delete the error code.

## 4. Detailed Design

### 4.1 Observation boundaries (the audit — Piece A)

A `Float` becomes user-accessible — and is therefore checked once via
`observe_float` — at exactly these NIR-level constructs (the check is **not**
emitted for anonymous operator results flowing between operators, nor for
register-allocator spills of temporaries):

1. **Binding / assignment** to a named `Float` local — `LET`, `MUT`, and `Assign`
   (including promoted loop accumulators: the check reads the promoted `d`-register
   after the update, so a named local is finite after every assignment).
2. **Collection store** — list append/set/insert and map insert of a `Float`.
3. **Record field store** — `WITH`/field assignment of a `Float`.
4. **`RETURN`** of a `Float`, and **passing a `Float` argument** to a FUNC/SUB.
5. **Observation** — `print`/`toString`/`toText`, and conversion `toInt`/`toByte`/
   `toFixed` of a `Float` (the convert path already range-checks; a non-finite must
   trap there too).
6. **Native FFI** — a `CDouble` return is already rejected if non-finite
   (language §17); a `Float` passed *out* to native must be finite → check.
7. **Thread transfer** — a `Float` crossing a thread boundary is observable on the
   other side → check at transfer-out.

Map keys compare bitwise and are a store boundary (1) — a non-finite key is
rejected at insert. The audit's deliverable is a checklist mapping each NIR
construct to its `observe_float` call site; **completeness is the gate**.

### 4.2 IEEE float comparisons (Piece B)

`fcmp` already sets NZCV per IEEE (unordered for `NaN`). Lower each comparison to
the condition that yields the required truth table without trapping:

| op | finite | `NaN` operand | `+Inf` vs finite x | `-Inf` vs finite x |
|----|--------|---------------|--------------------|--------------------|
| `<` `<=` `>` `>=` | IEEE | **false** | `>`/`>=` true, `<`/`<=` false | `<`/`<=` true, `>`/`>=` false |
| `=`  | IEEE | **false** (incl. `NaN=NaN`) | normal | normal |
| `<>` | IEEE | **true** | normal | normal |

This is plain IEEE: pick `b.<cond>` so unordered falls to the `false` side for
ordered predicates (and the `true` side for `<>`). Value `=`/`<>` use IEEE (so
`+0.0 = -0.0` is true, `NaN = NaN` false) — **map-key** equality stays bitwise and
is unchanged.

### 4.3 Removing `ErrFloatDomain` (Piece C)

- Delete the `float_compare_zero_d` divide-by-zero pre-check: `x/0` → `±Inf`,
  `0.0/0.0` → `NaN`, both caught at the boundary (`ErrFloatInf` / `ErrFloatNaN`).
- Remove the pow domain returns: `(-2)^0.5` → `NaN` (fdlibm), `0^-1` → `+Inf`,
  caught at the boundary.
- Delete `ERR_FLOAT_DOMAIN_CODE`/`_MESSAGE`/`_SYMBOL`, the `77050012` row in
  `src/spec/diagnostics/02_error-codes.md`, and any `errorCode::floatDomain`
  surface. Update `func_*` invalid tests that asserted `77050012`.

## Layout / ABI Impact

No value-layout change. `ErrFloatDomain` (`77050012`) is **removed** from
`mfb spec diagnostics` (the `errorCode::` build input) — a user-visible error-code
deletion. Native-code goldens change (the per-op checks vanish, boundary checks
appear) and are regenerated; `.run` goldens change **only** for programs that trap
on a non-finite (new location/code) — those are the intended behavior changes and
get re-baselined with care.

## Phases

1. **Spec.** Restate `04_types.md` §3 (finiteness at observation boundaries; IEEE
   comparison truth table; map-key bitwise carve-out) and remove the `77050012` row
   from `02_error-codes.md`. Acceptance: `mfb spec` renders; `errorCode::` builds
   without `floatDomain`.
2. **Boundary audit (no codegen change).** Produce the §4.1 checklist of NIR
   observation sites and the `observe_float` choke-point signature. Deliverable is
   the list; gate for Phase 4.
3. **IEEE comparisons.** Lower float `< > <= >= = <>` to the §4.2 truth table;
   finite comparisons byte-identical (golden-stable). Add `_valid` runtime cases
   that *would* compare a non-finite once Phase 4 lands (kept finite for now).
4. **Move the check + drop the `/0` pre-check (the core).** Remove the per-op
   `emit_float_result_check*` calls from `emit_float_binary`/pow; emit via
   `observe_float` at every Phase-2 boundary; delete the divide-by-zero pre-check.
   Targeted runtime proofs: `1.0/(1e200*1e200)` → `0`, **no trap**; `1e200*1e200`
   stored to a var → traps `ErrFloatOverflow`/`Inf` at the assignment line;
   `(1e200*1e200)-(1e200*1e200)` stored → `ErrFloatNaN` at the assignment.
5. **Delete `ErrFloatDomain`.** Remove the code path + constants + the pow domain
   returns + `errorCode::` entry; migrate the invalid tests.
6. **Validate + regenerate + measure.** Full suite; re-baseline the float-trap
   `.run`/`.ncode` goldens; confirm mandelbrot/nbody/leibniz ins-count and `c -O2`
   ratio drop (target: ~one check per assignment, not per op).

## Validation Plan

- Function tests: new `tests/func_*_valid/_invalid` for the moved trap points —
  transient-recover (no trap), escape-to-named-var (traps with the boundary's
  location/code), comparison truth table, divide-by-zero now `ErrFloatInf`/`NaN`.
- Runtime proof: float-nbody still `-0.169079859`, leibniz `pi: 3.14159`
  (finite-float outputs unchanged); the three §Phase-4 trap/recover programs.
- Doc sync: `mfb spec language types` and `mfb spec diagnostics error-codes`
  reflect the new rule and the removed code.
- Acceptance: full unfiltered `scripts/test-accept.sh`; native goldens regenerated;
  `.run` goldens change only for non-finite-trapping programs.
- Metric: per-benchmark ins count + `c -O2` ratio vs `benchmark/run.log`; expect
  the float bucket to move from per-op to per-assignment check density.

## Open Decisions

- **Boundary granularity** — check at *every named-variable assignment* (recommended:
  keeps "every named Float is finite" continuously true, simplest to audit, still
  ~one check per statement) vs. only at *escaping* observation (print/return/store-out;
  fewer checks but a named local could hold a non-finite between assignment and use,
  widening the leak surface). Recommend per-assignment. (§4.1)
- **Equality vs map keys** — value `=`/`<>` is IEEE (`NaN`≠`NaN`); map-key equality
  stays bitwise (`NaN`=`NaN`). This is an intentional, documented split (matches
  most languages); flag it in the spec so it is not read as a bug. (§4.2)

## Non-Goals

- A `d`-register-native float value model / `fmov`-shuttle removal — already done
  (plan-16 #1, committed `f45dec39`).
- Provably-finite *range* analysis to elide boundary checks (e.g. bounded-induction
  loops). A later optimization; this plan already removes the per-op cost.

## Summary

The engineering risk is concentrated in the §4.1 boundary audit: the check moves
from "everywhere, unconditionally" to "exactly the observation points," and missing
one lets a non-finite become user-accessible — the one thing the language forbids.
Get the boundary set complete and the rest is a relocation plus a truth table.
Finite-float behavior, value layout, copy/transfer, and map-key semantics are
untouched; the only intended behavior changes are *where* and *whether* a
non-finite-producing program traps — which is exactly the relaxation requested.

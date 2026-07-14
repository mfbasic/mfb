# MFBASIC Trap-Codegen Outlining & FP-Domain Float Checks Plan

Last updated: 2026-06-28

> **Status (2026-06-28).** Pieces A and B complete; only the explicit non-goal
> (#3 unchecked-float mode) remains, deferred to its own future plan.
> - **#2 ErrorLoc outlining — DONE, committed `1a2f10db`.** `_mfb_build_error_loc`
>   helper; each trap site ~48 instrs → ~5.
> - **"Option 2" make-error-result outlining — DONE, committed `3bace43b`.**
>   `_mfb_make_error_result` helper; the rest of the per-site Result shuffle is now
>   a call. Cumulative ins-count vs `benchmark/run-old3.log`: nbody −60%
>   (100,319→40,048), parse-regex −37% (847,459→530,004), parse-json −31%,
>   leibniz −33%. Full suite 972 green, zero behavior mismatches; runtime
>   error-location tests byte-identical.
> - **#1 FP-domain check / d-native floats — DONE (Piece B), committed in two
>   steps.** Landed exactly as the audit recommended (FP-domain check + a
>   liveness-based peephole; the GP-native value model was left untouched).
>   - *Step 1 — FP-domain finiteness check.* New scalar `fabs d` (`FAbsD`) and
>     `b.vs` (`BranchVs`) ops (encoders/ABI/CFG/peephole-model). `emit_float_result_check_fp`
>     checks the result while it is still in its `d`-register: `fabs` folds ±Inf,
>     one `fcmp` against +Inf orders finite (`<`) / inf (`==`) / NaN (unordered, V →
>     `b.vs`) — the same 3-way predicate, same codes/messages, byte-identical
>     line:char. Routed Float `+ - * / MOD ^`. As predicted this is ~instruction-
>     neutral on its own (it frees the GPR `dst` to be store-only).
>   - *Step 2 — FP-shuttle peephole.* `peephole::remove_fp_shuttles` (after
>     `forward_stores_to_loads`, on the colored stream) drops the now-dead shuttle:
>     `fmov xN,dM; str xN,[slot]` → `str d dM,[slot]` and the `ldr`+`fmov` inverse,
>     gated on integer **live-out** (`regalloc::integer_live_out`, call clobbers
>     modeled as kills) proving `xN` is dead. Identical 64 bits, so a `str d`-written
>     slot reloaded by `ldr x` stays correct.
>   - *Result.* Float result/operand shuttles in the loops collapse (leibniz
>     `fmov_x_from_d` 8→2); values store straight from their `d`-register. Static
>     ins-count (vs the Piece-A-complete binary): nbody 40,383→39,619, leibniz
>     1,385→1,371, mandelbrot 2,318→2,274 — the win is float residency, not static
>     size (the check itself is ~neutral). Full suite 972 green; every float trap
>     (overflow/nan/domain/mod/pow) fires with the same code/message/location;
>     nbody still prints -0.169079859, leibniz pi: 3.14159.
> - **#3 elide checks / unchecked-float mode — DEFERRED** (changes when traps fire;
>   its own future plan).

Fallible scalar operations dominate MFBASIC's native code size and float-loop
runtime — not because the arithmetic is expensive, but because every trapping op
emits, **inline at the call site**, a ~48-instruction block that allocates an
`ErrorLoc`, copies the source filename, and stamps the line/column. Measured
share of the binary that is this one construction: float-nbody **~51%**,
parse-json ~34%, parse-regex ~30% (5,450 sites × ~48 instrs ≈ 260K instructions),
float-leibniz ~39%. None of it executes on the success path; it is pure static
bloat plus, for floats, a forced `fmov` of every result through a GP register so
the inline NaN/Inf check can bit-test it.

This plan makes two behavior-preserving codegen changes that shrink the binary and
the float hot path, with **identical observable semantics** (same errors, same
messages, same locations): (1) move the `ErrorLoc` construction into a single
shared runtime helper; (2) perform the float finiteness check in the FP domain so
float values stay resident in `d`-registers. A third, semantics-changing idea
(eliding checks / an unchecked-float mode) is explicitly deferred.

It complements:

- `./mfb spec diagnostics error-codes` (the `ErrorLoc`/`Error` records and trap
  semantics this preserves; canonical specs live under `src/spec/**`)
- `./mfb spec memory` (the flat `Error`/`ErrorLoc` layout — unchanged here)

## 1. Goal

- Replace the inline `emit_build_error_loc` block (builder_codegen_primitives.rs:226)
  at every trap site with a call to a new runtime helper
  `_mfb_build_error_loc(filename, line, char) -> ErrorLoc*` (null on OOM). Each
  site drops from ~48 instructions to ~5 (load filename constant, `mov` line/col,
  `bl`). One copy of the body lives in the runtime.
- Perform the float non-finite (NaN/Inf) trap check with FP-domain instructions
  (`fabs`/`fcmp` against +Inf, `fcmp d,d` for unordered) instead of
  `fmov`-to-GP + integer bit-test, so a float result is checked without leaving
  the `d`-register and can be stored with `str d` / kept resident.
- Net target: float-nbody and parse-regex shrink ≥25% in instruction count;
  all existing acceptance and error tests pass byte-for-byte in **behavior**
  (native-code goldens are expected to change and get regenerated).

### Non-goals (explicit constraints)

- **No change to observable error behavior.** Same error codes, messages, and
  source locations (`filename`/`line`/`char`) for every trap, on every path,
  including OOM-degraded errors (null `ErrorLoc`). This is the guardrail.
- **No change to the `Error`/`ErrorLoc` record layout or to value/copy/transfer
  semantics** (`mfb spec memory`). The helper produces a byte-identical
  `ErrorLoc` to today's inline code.
- **No change to which operations trap or when** (that is the deferred #3). The
  float check computes the same predicate; only the instructions differ.
- No change to the language surface or to the register-allocator's correctness
  contract (plan-03).

## 2. Current State

- `emit_build_error_loc` (src/target/shared/code/builder_codegen_primitives.rs:226)
  is emitted inline at each trap site by `emit_error_register_return` /
  `emit_error_code_return` and the thread-error paths. It allocates an `ErrorLoc`
  (`{filename@0, line@8, char@16}`, fixed 24 bytes + inlined filename String
  block), copying `self.current_file` and stamping `self.current_loc`. It returns
  the pointer in `x9`, null on OOM, and is documented as allocation-pool-free and
  terminal.
- Runtime helper functions are `CodeFunction`s built by `lower_*` routines
  (src/target/shared/code/entry_and_arena.rs:575 `lower_arena_alloc`) and pushed
  unconditionally into `code_functions` (src/target/shared/code/mod.rs:591). The
  new helper mirrors this exactly: a `lower_build_error_loc` pushed alongside.
- The float finiteness check is the `fmov xN, dM; lsl; mov/movk #0xffe0…; cmp;
  b.lo/b.hi` sequence emitted after each fallible float op (observed in
  float-leibniz at the `fdiv` site). It forces every float result into a GP
  register, which is why floats round-trip through the stack as integer bits
  (13 `fmov` shuttles per leibniz iteration for 5 float ops).
- `ERROR_LOC_OBJECT_SIZE` and `ARENA_ALLOC_SYMBOL` are in
  src/target/shared/code/error_constants.rs.

## 3. Design Overview

Two independent pieces, landable separately, lowest-risk first:

- **Piece A — outline `ErrorLoc` construction (the big win).** A pure relocation
  of existing logic into one runtime function. Correctness risk is the helper's
  calling convention and that it reproduces the exact `ErrorLoc` bytes incl.
  OOM→null. Verified by: error tests unchanged in behavior + instruction-count
  drop.
- **Piece B — FP-domain float check.** Recompute the same non-finite predicate
  with `fabs`/`fcmp`, leaving the value in its `d`-register. Correctness risk is
  matching today's predicate exactly (which bit patterns trap). Verified by: the
  float `_invalid` tests (overflow/NaN traps) still fire identically, and the
  `fmov`-shuttle count in float loops drops toward zero.

Piece A is strictly bigger and strictly lower-risk, so it lands first. Piece B
builds on the smaller post-A loop body.

## 4. Detailed Design

### 4.1 Piece A — `_mfb_build_error_loc` runtime helper

Signature (AArch64): `x0 = filename String*` (never null; empty-String constant
when the file is unknown), `x1 = line`, `x2 = char`. Returns `x0 = ErrorLoc*`, or
`x0 = 0` on allocation failure. Clobbers caller-saved only; preserves callee-saved
(standard PCS — the register allocator already models `_mfb_*` helpers as
clobbering caller-saved + integer scratch, plan-03).

Body (the current `emit_build_error_loc` logic, reading registers instead of
compile-time constants), built as a `CodeFunction`:
1. Prologue: `sub sp; str x30` and save `x1`/`x2`/`x0` across the alloc call.
2. `len = *filename`; `size = ERROR_LOC_OBJECT_SIZE + len + 9`; `bl _mfb_arena_alloc`
   (size in x0, align 8 in x1).
3. On `RESULT_OK_TAG`: write `{ERROR_LOC_OBJECT_SIZE@0, line@8, char@16}`, then
   inline-copy the filename block (`len + 9` bytes) at offset
   `ERROR_LOC_OBJECT_SIZE` via the shared byte-copy.
4. On OOM: return null.
5. Epilogue.

Call-site rewrite in `emit_build_error_loc`: resolve the filename String constant
into `x0` (the existing `load_empty_string_constant` / `emit_load_string_constant`
paths), `mov x1, #line`, `mov x2, #char`, `bl _mfb_build_error_loc` (+ internal
relocation), `mov x9, x0`. The function keeps returning `x9`, so **all callers are
unchanged**. Register-input-saving contract is unchanged (callers still save live
inputs before the now-`bl`).

Registration: `BUILD_ERROR_LOC_SYMBOL = "_mfb_build_error_loc"` in
error_constants.rs; `code_functions.push(lower_build_error_loc(platform)?)` in
mod.rs next to `lower_arena_alloc`. Always present (every program can trap); if a
dead-strip pass exists it may drop it when no site references it.

### 4.2 Piece B — FP-domain non-finite check

Replace `fmov xN,dM; lsl #1; cmp #0xffe0…<<48; b.lo/b.hi` with: keep the result in
`dM`; `fcmp dM, dM` catches NaN (unordered → V/`vs`); compare `fabs(dM)` against a
materialized `+Inf` (`fmov dInf, #…` or a constant load) with `fcmp` to catch
overflow/Inf. Branch to the (now-outlined) trap helper on the failing condition.
The predicate is identical to the integer bit-test (`|bits|<<1 ≥ 0xffe0…` is
exactly "exponent all-ones" = Inf or NaN); B only changes the instructions used to
evaluate it. With no `fmov`-to-GP, the value is stored with `str d` and stays
eligible for FP residency/promotion (plan-03 Stage C/D), removing the GP/stack
round-trips.

## Layout / ABI Impact

None to value layout. The `Error`/`ErrorLoc` records (`mfb spec memory`) are
byte-identical. New **internal** runtime symbol `_mfb_build_error_loc` (not a
language-visible surface). Native-code goldens change (instruction sequences) and
are regenerated; `.run`/behavioral goldens must **not** change.

## Phases

1. **Audit + helper, no callers (Piece A.1).** Add `BUILD_ERROR_LOC_SYMBOL`,
   `lower_build_error_loc`, register it in `code_functions`. Helper present but
   unused. Acceptance suite still green (dead function, zero behavior change).
2. **Switch trap sites to the helper (Piece A.2).** Rewrite `emit_build_error_loc`
   to call it. Acceptance suite behavior-green; native goldens regenerated;
   instruction-count drop measured vs `benchmark/run-old3.log` (the ins-count
   baseline — its timings are 1-run noise and must be ignored).
3. **FP-domain float check (Piece B). DONE.** Behind the smaller post-A loop body.
   Float `_invalid` traps fire identically; `fmov`-shuttle count in float loops
   drops (leibniz 8→2); ins-count measured (nbody 40,383→39,619). Landed as two
   commits — the FP-domain check, then the liveness-based shuttle peephole.

## Validation Plan

- Function/behavior tests: every `tests/*_invalid/**` that asserts a trap
  (overflow, divide, NaN, bounds, OOM-degraded) must produce the **same** error
  code, message, and `file:line:char` as before. The error-location tests are the
  core proof.
- Runtime proof: float-nbody still prints `-0.169079859` (matches the C
  reference); a program that traps mid-expression reports the unchanged location.
- Doc sync: none expected (no error-code or layout change); confirm `mfb spec
  diagnostics` still matches.
- Acceptance: full unfiltered `scripts/test-accept.sh target/debug/mfb
  target/accept-actual` behavior-green; regenerate only native-code goldens.
- Metric: instruction-count deltas per benchmark vs `run-old3.log` (ins column
  only). Target ≥25% on float-nbody and parse-regex after Phase 2.

## Open Decisions

- **Filename argument vs. global.** Pass the filename String pointer per site
  (recommended — supports multi-file programs where `current_file` varies, and the
  constant is already materialized at the site) vs. a single program-wide global
  (smaller call sites but wrong for multi-unit builds). Recommend per-site.
- **Phase 3 inclusion now vs. later.** Recommend landing Phases 1–2 first (the
  bulk of the win, lowest risk) and Phase 3 immediately after on the smaller body.

## Non-Goals

- **#3: eliding checks / unchecked-float mode.** Changes *when* traps fire —
  a visible-behavior/spec change. Deferred to its own plan; it is the riskiest of
  the three and not required for the code-size and float-residency wins here.

## Summary

The engineering risk is concentrated in Piece A's helper reproducing the exact
`ErrorLoc` bytes and OOM→null behavior, and Piece B matching the trap predicate
bit-for-bit. Both are behavior-preserving by construction and fall out of existing
precedents (`lower_arena_alloc`; the existing finiteness predicate). Everything
about value layout, copy/transfer semantics, and which operations trap stays
untouched — the only thing that changes is how many instructions it takes to say
the same thing.

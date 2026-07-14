# plan-26-A: Front-end uniformity + infallible inline-TRAP passthrough

Last updated: 2026-07-06
Overall Effort: large (3h–1d)
Effort: medium (1h–2h)

Make inline `TRAP` (and `RECOVER`) legal on **every** callable, so that to the
developer a built-in call is just a function call. Today the front-end rejects
`TRAP` on inline-lowered built-ins with `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`
(infallible members) and `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` (fallible members
without a raw lowering). Both diagnostics leak an implementation detail —
inline-vs-real-call — into the language surface. This sub-plan removes the
*infallible* leak: a `TRAP` on a provably-infallible built-in must compile and be
harmless dead code (the handler never fires, exactly as a `TRAP` on any
infallible user FUNC would behave), and codegen must produce an always-`Ok`
`Result` for it so the inline-TRAP machinery has something to trap.

The single behavioral outcome: `x = TRAP len(list) BINDING e ... RECOVER 0`
compiles, runs, returns `len(list)`, and never enters the handler. No diagnostic.

It complements:

- `./mfb spec language error-model` and `./mfb spec language pattern-matching`
  (inline TRAP / RECOVER surface; canonical specs under `src/docs/spec/**`).
- `./mfb spec diagnostics error-codes` (`src/docs/spec/diagnostics/02_error-codes.md`
  — the `2-203-00xx` rows this plan retires).

## 1. Goal

- Inline `TRAP`/`RECOVER` on a provably-infallible inline built-in
  (`len`, `toString`, `typeName`, all `bits::*`, and the infallible collection/map
  members `contains`/`hasKey`/`keys`/`values`/`sum`/`getOr`/`append`/`prepend`/
  `removeKey`/`replace`) compiles with **no** diagnostic and runs correctly with a
  dead handler.
- `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` (`2-203-0069`) is retired from the inline-TRAP
  path (kept only for non-call / package-constant scrutinees).
- A new advisory `Warning`-severity rule (`TYPE_INLINE_TRAP_DEAD_HANDLER`) fires when
  a `TRAP` guards a provably-infallible inline built-in — the program still compiles
  and runs; the warning flags the unreachable handler.
- The still-standing handler well-formedness rule `TYPE_INLINE_TRAP_FALLS_THROUGH`
  (`2-203-0066`, handler must RECOVER or diverge) and `TYPE_RECOVER_TYPE_MISMATCH`
  (`2-203-0067`) are unchanged.

### Non-goals (explicit constraints)

- No change to language surface beyond *removing* a rejection: `TRAP`/`RECOVER`
  syntax, MATCH, binding forms all unchanged.
- No change to value/copy/move/drop semantics or to golden native output for any
  program that does **not** newly add such a TRAP. Success fall-through of an
  infallible built-in must be byte-identical to today when not under a TRAP.
- The fallible-but-unsupported callback members (`forEach`/`transform`/`filter`/
  `reduce`) stay rejected here — they are plan-26-B. This sub-plan does **not**
  touch `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`.
- `expectTrap`/`expectNTrap` parity is plan-26-C.

## 2. Current State

- Front-end gate: `src/syntaxcheck/inference.rs:97-140`. `fallible` is computed as
  `!is_package_constant && !inline_builtin_is_infallible`; when `!fallible` it emits
  `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` (`inference.rs:113-120`).
- Predicates: `src/builtins/mod.rs:171` (`inline_trap_unsupported`), `:189`
  (`inline_builtin_raw_supported`), `:215` (`inline_builtin_is_infallible`).
- Codegen raw path: `src/target/shared/code/builder_values.rs:764` routes
  `inline_builtin_raw_supported` targets to `lower_inline_builtin_raw`
  (`builder_values.rs:1363`), which sets `raw_result_capture`, lowers the member,
  and on success fall-through tags the value `Ok` and materializes `Result OF
  <success>`. The success shape is a single register (`builder_values.rs:1384-1393`).
- Error capture join: `emit_error_register_return`
  (`builder_codegen_primitives.rs:737`) branches to `raw_result_capture` when set —
  this is the mechanism a TRAP relies on. An infallible member emits **no** call to
  `emit_error_register_return`, so under a capture it simply reaches the success
  fall-through.
- Backstop: `builder_values.rs:781` still guards `inline_trap_unsupported` at codegen.

## 3. Design Overview

Two small changes that layer:

1. **Front-end (`inference.rs`):** stop treating "infallible" as a rejection.
   Infallible built-ins become *allowed* under a TRAP; the handler is simply dead.
   The `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` branch for genuinely-unsupported
   fallible members (the callback set) stays until plan-26-B lands.

2. **Codegen (`builder_values.rs`):** widen the raw-lowering entry so an infallible
   inline built-in *invoked under a TRAP* is lowered through the same
   `raw_result_capture` wrapper and tagged always-`Ok`. Because the member emits no
   error exit, this is just "set capture, lower normally, tag Ok, materialize
   Result" — the existing `lower_inline_builtin_raw` shape already does exactly this
   for the single-register success case; the change is which targets are allowed to
   reach it.

Correctness risk is low and concentrated in the success-shape assumption: the
always-Ok wrapper must only fire when a TRAP actually wraps the call (otherwise a
bare `len(x)` would start materializing a `Result` and break golden output). The
wrapper must key off "is this call the direct child of a `Trapped` expression",
not off the built-in identity.

## 4. Detailed Design

### 4.1 Front-end gate (`src/syntaxcheck/inference.rs`)

- Change the `fallible` computation so an **infallible inline built-in no longer
  produces `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`**. Keep the diagnostic only for
  cases with genuinely nothing to trap that are also *not* inline built-ins — i.e.
  package constants (`is_package_constant`) and literals/non-calls. Concretely:
  - Non-call scrutinee → still `REQUIRES_FALLIBLE` (unchanged).
  - Package-constant call → still `REQUIRES_FALLIBLE` (a constant is not a call in
    the runtime sense).
  - Infallible inline built-in call → **no error**; emit the advisory
    `TYPE_INLINE_TRAP_DEAD_HANDLER` `Warning` (new) — the call compiles and runs, the
    handler is dead.
  - Fallible inline built-in without raw support (callback set) →
    `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` (unchanged; retired in 26-B).
- `RECOVER` type-checks against the success type as today; a dead handler still must
  be well-formed (`TYPE_INLINE_TRAP_FALLS_THROUGH`, `TYPE_RECOVER_TYPE_MISMATCH`
  unchanged).

### 4.2 Codegen always-Ok wrapper (`src/target/shared/code/builder_values.rs`)

- The builder already knows it is lowering a trapped call (the raw path is entered
  only under a TRAP). Extend the target set that reaches `lower_inline_builtin_raw`
  to include infallible inline built-ins **when a raw capture is being requested**
  (i.e. the trapped-call lowering path), not on the ordinary call path.
- Add infallible dispatch arms to `lower_inline_builtin_raw`
  (`builder_values.rs:1371`): lower the member normally (it cannot fail), then the
  existing fall-through tags the single success register `Ok` and materializes
  `Result OF <success>`. `len`/`find`-style produce an Integer, `toString`/`typeName`
  a String, the query members their documented type — all single-register, so the
  existing success shape covers them.
- Guard: on the **non-trapped** path, infallible built-ins lower exactly as today
  (no capture, no Result materialization) so golden output is untouched.
- Update the `builder_values.rs:781` backstop so it no longer rejects an infallible
  built-in (it must only reject the still-unsupported callback set).

## Layout / ABI Impact

None. No struct, record, `.mfp`, or register-model change. Native output for any
program not newly adding an infallible-built-in TRAP is byte-identical (verified via
the artifact gate).

## Phases

### Phase 1 — front-end: allow infallible inline built-ins under TRAP

Retire the infallible-side rejection; keep every other rule.

- [x] `src/syntaxcheck/inference.rs:97-120`: recompute `fallible`/reporting so an
      infallible inline built-in call under a `Trapped` node emits no **error** and
      instead emits the advisory `TYPE_INLINE_TRAP_DEAD_HANDLER` `Warning`; non-call
      and package-constant scrutinees still emit `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`.
- [x] `src/rules/table.rs`: add `TYPE_INLINE_TRAP_DEAD_HANDLER` as a
      `Severity::Warning` row (next free `2-203-00xx` code) with message
      "inline TRAP handler is unreachable — the guarded call cannot fail".
- [x] Keep the `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` branch (`inference.rs:127-140`)
      exactly as-is for the callback set.
- [x] Update/added inference unit tests near `inference.rs:1701-1716` to assert the
      infallible cases now *pass with the warning*, and the non-call/constant cases
      still reject with the error.

Acceptance: a source file with `TRAP len(list) ... RECOVER 0` passes syntax check
with the `TYPE_INLINE_TRAP_DEAD_HANDLER` warning (no error); `TRAP <literal>` and
`TRAP <package-constant>` still reject with `..._REQUIRES_FALLIBLE`; callback-member
TRAP still rejects with `..._ON_INLINED_BUILTIN`.
Commit: —

### Phase 2 — codegen: always-Ok materialization for infallible built-ins under TRAP

Give the inline-TRAP machinery a Result to trap; leave the non-trapped path untouched.

- [x] `src/target/shared/code/builder_values.rs:764-781`: route infallible inline
      built-ins to `lower_inline_builtin_raw` **only** on the trapped-call path;
      relax the `:781` backstop to reject only the callback set.
- [x] `src/target/shared/code/builder_values.rs:1371-1381`: add dispatch arms for the
      infallible members (`len`, `toString`, `typeName`, `bits::*`, and the infallible
      collection/map queries) that lower normally and fall through to the Ok tag.
- [x] Confirm the non-trapped lowering of every one of those built-ins is unchanged
      (no capture, no Result) — inspect the ordinary call arms are not diverted.

Acceptance: artifact gate (`scripts/artifact-gate.sh`) shows byte-identical native
output for the whole tree except the new/changed inline-TRAP fixtures; a runtime
program printing `TRAP len(list) BINDING e RECOVER -1` outputs the real length.
Commit: —

### Phase 3 — fixtures + spec

- [x] `tests/func_inline_trap_infallible_valid/**`: TRAP over a representative
      infallible built-in per family (`len`, `toString`, `bits::sl`, `contains`,
      `getOr`), each with a runtime proof the handler is dead and the value is correct;
      the build log carries the `TYPE_INLINE_TRAP_DEAD_HANDLER` warning (a warning does
      not fail the build).
- [x] `tests/func_inline_trap_infallible_invalid/**`: TRAP over a literal and over a
      package constant still reject with `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`.
- [x] `src/docs/spec/diagnostics/02_error-codes.md`: add the
      `TYPE_INLINE_TRAP_DEAD_HANDLER` warning row and narrow the
      `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` (`2-203-0069`) description to
      non-call/package-constant scrutinees (it no longer fires for infallible
      built-ins). Do **not** delete any code yet — `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`
      retirement and any full removal is plan-26-B/C.
- [x] `src/docs/spec/language/error-model.md`: note that TRAP is legal on any call;
      on an infallible callee the handler is dead.

Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` green;
`mfb spec diagnostics error-codes` reflects the narrowed rule.
Commit: —

## Validation Plan

- Function tests: `tests/func_inline_trap_infallible_valid/**` + `_invalid/**`.
- Runtime proof: a program that TRAPs `len`/`toString`/`bits::sl` and prints both the
  value and a marker if the handler ran — the marker never prints; the build emits the
  `TYPE_INLINE_TRAP_DEAD_HANDLER` warning and still succeeds.
- Doc sync: `mfb spec diagnostics error-codes`, `mfb spec language error-model`.
- Acceptance: `scripts/test-accept.sh`; codegen non-regression via
  `scripts/artifact-gate.sh` (per [[fast-codegen-gate]]).

## Open Decisions

- **Warn on a provably-dead TRAP handler? — DECIDED: yes, warn.** Emit an advisory
  `Warning`-severity rule when a `TRAP` (or `RECOVER`) guards a provably-infallible
  call — the call compiles and runs, but the handler is unreachable dead code, and
  the developer should be told. This is a `Severity::Warning`, not an `Error`: it
  never rejects the program (uniformity is preserved — the call is still legal), it
  only flags the dead handler. Only fires where the compiler *proves* infallibility
  (`is_package_constant` is excluded — those still hard-reject as non-calls;
  `inline_builtin_is_infallible` is the trigger). It does **not** fire for an
  infallible *user* FUNC (the compiler can't prove that today), which is acceptable —
  the warning covers exactly the built-ins whose infallibility is known.

## Non-Goals

- Callback-member inline-TRAP (plan-26-B).
- `expectTrap`/`expectNTrap` parity + full rule retirement (plan-26-C).

## Summary

Low-risk: removes a rejection and reuses the existing single-register raw-success
shape. The only real trap is diverting infallible built-ins to Result-materialization
on the *non*-trapped path — guarded by keying the wrapper off the trapped-call site,
verified byte-identical by the artifact gate.

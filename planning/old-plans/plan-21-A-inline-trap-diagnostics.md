# plan-21-A: Inline-TRAP diagnostic classification

Last updated: 2026-07-03
Overall Effort: large (3h–1d)
Effort: small (<1h)

Make the inline-`TRAP` diagnostics tell the truth about *why* a trap is rejected.
Today any inline TRAP on an inline-lowered built-in — fallible or not — is
reported as `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` with the remedy "move it into a
FUNC/SUB and TRAP there." For an infallible built-in (`len`, `toString`,
`typeName`, `bits::*`, pure-query members) that remedy is misleading: wrapping
the call traps nothing. This sub-plan classifies built-in callees by fallibility
and routes infallible ones to the already-accurate
`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` ("this expression cannot fail"), leaving
`TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` for the genuinely-fallible inline members
that [plan-21-B](plan-21-B-inline-trap-raw-lowering.md) will later enable.

The single behavioral outcome: `LET n = len(xs) TRAP(e) … END TRAP` reports
`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`, while `LET v = collections::get(xs, i)
TRAP(e) … END TRAP` still reports `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` (until B).

It complements:

- `./mfb spec language error-model` (§8.6 rules 11 & 14;
  `src/docs/spec/language/error-model.md`)
- `./mfb spec diagnostics error-codes` (`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`,
  `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` `2-203-0102`)

## 1. Goal

- A fallibility classifier for built-in callees that both this sub-plan (diagnostic
  routing) and plan-21-B (which members get raw lowerings) consume.
- Inline TRAP on an infallible built-in → `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`.
- Inline TRAP on a fallible inline member → `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`
  (unchanged, narrowed to this set).

### Non-goals (explicit constraints)

- No new capability — no inline TRAP is *enabled* here; both messages are still
  compile errors. Enablement is plan-21-B.
- No change to inline-TRAP placement/handler rules or to how user `FUNC`/`SUB`
  calls (which carry real symbols) are classified — they remain fallible and
  trappable.
- No codegen change; this sub-plan touches only typecheck, the built-ins
  classifier, the rules table, and the spec/diagnostics docs.

## 2. Current State

- Gate: `src/typecheck/inference.rs:90-142`. `fallible =
  !builtins::is_package_constant(canonical)` (line 102-105) — treats every
  non-constant call as fallible, so `len` reaches the inline-built-in branch
  (line 129-141) and gets `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`. The accurate
  `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` (line 115-122) fires only for constants.
- Unsupported set: `src/builtins/mod.rs:170-174` (`inline_trap_unsupported`) over
  `bits::is_bits_call`, `native_builtin_target` (`mod.rs:114-145`), and
  `len`/`toString`/`typeName`.
- Fallibility signals already in the codebase: `value_may_return_invalid_format`
  (`src/target/shared/code/module_analysis.rs:869`) for the conversion built-ins;
  per-member error emission via `emit_error_code_return`
  (`builder_codegen_primitives.rs:301`) — e.g. `lower_collection_get`
  (`builder_collection_queries.rs:25`) raises `ERR_INDEX_OUT_OF_RANGE_CODE`,
  `lower_find` raises `77050001` on negative start (`builder_search.rs:158`).

## 3. Design Overview

Add `builtins::inline_builtin_is_infallible(canonical) -> bool` in
`src/builtins/mod.rs` next to `inline_trap_unsupported`, sharing its canonical-name
contract. It returns `true` for callees that can raise no user-trappable domain
error:

- `len`, `toString`, `typeName`, all `bits::*`.
- Pure-query / default-returning members: `contains`, `hasKey`, `keys`, `values`,
  `sum`, `getOr`, `isEmpty`, `isNotEmpty`, and any member that finishes census as
  raising only OOM (see Open Decision in the umbrella: OOM is not trappable).

The gate then computes fallibility as
`!is_package_constant(c) && !inline_builtin_is_infallible(c)` so infallible
built-ins fall into the existing `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` branch. The
`TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` branch is unchanged but now only reached by
the fallible inline members.

The correctness hinge is the census: mis-classifying a fallible member as
infallible would wrongly report "cannot fail." Ground each entry in the member's
`lower_*` method — infallible iff it has no `emit_error_code_return` /
`emit_*_out_of_range` on any success-relevant path (OOM-only counts as infallible
per the Open Decision).

## Phases

### Phase 1 — Fallibility census + classifier

Produce the authoritative fallible/infallible split and encode it once.

- [ ] Audit each `native_builtin_target` member (`src/builtins/mod.rs:114-145`)
      against its `lower_*` method in `src/target/shared/code/*` and record
      fallible vs infallible (domain error present vs OOM-only/none). Capture the
      table in this doc under a "Census" heading.
- [ ] Add `inline_builtin_is_infallible(canonical: &str) -> bool` to
      `src/builtins/mod.rs`, covering `len`/`toString`/`typeName`, all `bits::*`
      (`bits::is_bits_call`), and every infallible member from the census.
- [ ] Add a unit test in `src/builtins/` (or the nearest existing builtins test
      module) asserting the classifier agrees with the census for every inline
      member — the guard against a future member drifting.

Acceptance: `cargo test` covers the classifier; the census table lists every
inline member with a fallibility verdict cited to its lowering method.
Commit: —

### Phase 2 — Diagnostic routing + doc sync + tests

Wire the classifier into the gate and update the specs.

- [ ] In `src/typecheck/inference.rs:102-105`, fold
      `inline_builtin_is_infallible` into the `fallible` computation so infallible
      built-ins route to `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`.
- [ ] Confirm `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` exists in `src/rules/table.rs`
      with the right severity/message; refine the message if the census surfaces a
      clearer wording for the built-in case.
- [ ] Update `src/docs/spec/language/error-model.md` §8.6: rule 11 (infallible
      inline built-ins now say "cannot fail") and rule 14 (narrow "inline-lowered"
      rejection to the fallible members; note the conversion built-ins already
      support it and B extends that to the fallible members).
- [ ] Update `src/docs/spec/diagnostics/**` error-codes prose for both codes.
- [ ] Tests: `tests/func_*_invalid/**` goldens — one asserting `len(xs) TRAP(e)`
      yields `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`, one asserting a fallible member
      (`collections::get(xs, i) TRAP(e)`) still yields
      `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`.

Acceptance: the two `_invalid` goldens show the correctly-routed codes and
`scripts/test-accept.sh target/debug/mfb target/accept-actual` passes.
Commit: —

## Validation Plan

- Function tests: `_invalid` diagnostic goldens for the infallible route and the
  still-rejected fallible route.
- Doc sync: error-model §8.6 rules 11 & 14; diagnostics error-codes.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- OOM-only members classified infallible-for-TRAP (see umbrella Open Decisions).
  Finalized by this sub-plan's census.

## Summary

Low-risk front-end reroute. The only way to get it wrong is a bad census entry,
so every verdict is cited to a lowering method and locked by a classifier unit
test. Produces the census plan-21-B consumes.

# plan-21-B: Inline-TRAP raw lowering for fallible inline members

Last updated: 2026-07-03
Effort: medium (1h–2h)

Enable inline `TRAP` on the fallible inline-lowered members
(`collections::get`, `strings::mid`, `collections::set`/`insert`/`removeAt`,
`find`, … — the exact set is plan-21-A's census). Today these are rejected with
`TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` even though they fail at runtime, while the
sibling conversion built-ins (`toInt`, `toFloat`, …) — equally inline-lowered —
support inline TRAP through `lower_inline_conversion_raw`. This sub-plan
generalizes that one working mechanism to cover the fallible members.

The single behavioral outcome: `LET v = collections::get(xs, i) TRAP(e) RECOVER
default … END TRAP` compiles, and at runtime an out-of-range `i` runs the handler
(recovering the default) instead of propagating/aborting — exactly as a trapped
`toInt` or a trapped user `FUNC` does.

Depends on **plan-21-A** (the fallibility census: which members are in scope).

It complements:

- `./mfb spec language error-model` (§8.4 inline TRAP, §8.8 desugaring;
  `src/docs/spec/language/error-model.md`)
- `./mfb spec memory fallible-call-abi` (the `Result` register ABI the raw path
  materializes into; unchanged here)

## 1. Goal

- A member-agnostic raw-`Result` inline lowering that wraps any fallible inline
  member's normal `lower_*` method, mirroring `lower_inline_conversion_raw`.
- The census's fallible members removed from `inline_trap_unsupported` so the gate
  admits them and codegen routes them to the raw wrapper.
- `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` becomes unreachable from the front-end
  (no fallible inline member is left in the unsupported set) and remains only as
  the codegen backstop.

### Non-goals (explicit constraints)

- No inline TRAP on infallible built-ins — `len`/`toString`/`typeName`/`bits::*`
  and pure-query members stay rejected via plan-21-A's
  `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`. They own no raw wrapper.
- No layout/ABI change and no change to untrapped call sites — the raw wrapper
  fires only when a TRAP is attached; every existing golden binary stays
  byte-identical.
- No change to the handler contract (`RECOVER`/diverge, rule 12) or to
  value/copy/move/drop on the error path.

## 2. Current State

- Working precedent: `lower_inline_conversion_raw`
  (`src/target/shared/code/builder_values.rs:1226-1259`) — sets
  `raw_result_capture`, dispatches by target to the built-in's `lower_*`, tags the
  success value `Ok` on fall-through, lands at the capture label, and calls
  `materialize_current_result` (`builder_arena_transfer.rs:78`). Reached from
  `builder_values.rs:666-694` for `toInt`/`toFloat`/`toFixed`/`toByte`.
- Generic error hinge (already in place): `emit_error_register_return`
  (`builder_codegen_primitives.rs:667`) branches to `raw_result_capture` instead
  of `return_()` whenever a capture is active. Every fallible member raises
  through `emit_error_code_return` (`builder_codegen_primitives.rs:301`), so the
  error paths already cooperate with a capture — no per-member error-path edits.
- Member dispatch: `builder_values.rs:516-593` maps each `native_builtin_target`
  result to its `lower_*` method (`lower_collection_get`
  `builder_collection_queries.rs:25`, `lower_mid` `builder_search.rs:473`,
  `lower_collection_set`/`_insert`/`_remove_at`, `lower_replace`
  `builder_strings.rs:4`, `lower_find` `builder_search.rs:4`).
- Gate/backstop: `inline_trap_unsupported` (`src/builtins/mod.rs:170-174`) and the
  codegen backstop in `builder_values.rs` (the "front-end gate should have
  rejected it" error).

## 3. Design Overview

Generalize the conversion-only raw path into a member-agnostic wrapper. Where the
untrapped call dispatches `native == Some("get") => lower_collection_get(args)`,
the trapped call runs the same `lower_*` under a raw capture. The success
fall-through must handle heterogeneous member result shapes:

- **Value-returning** (`get`, `mid`, `find`, `replace`): tag the returned
  location `Ok` and materialize `Result OF <success_type>` — identical to the
  conversion path.
- **Mutating, collection-returning** (`set`, `insert`, `removeAt`, `removeKey`):
  the `lower_*` returns the updated collection pointer; same treatment, success
  type is the collection type.
- **`Nothing`-typed** (if any fallible member returns nothing): materialize
  `Result OF Nothing` with no value register — the same case the value-less
  helper trap already handles (`lower_runtime_helper_call(..., raw=true)`).

The wrapper keys on `builtins::call_return_type_name` for the success type (as the
conversion path does) and dispatches to the member's `lower_*` by the same
`native_builtin_target` name the normal path uses, so the two paths cannot drift.

Correctness risk: a member whose success path emits its own early `return_()`
(rather than falling through to the wrapper's Ok-tag) would bypass the capture
join and miscompile. Phase 2 audits each in-scope member for a clean
fall-through success and enables them one at a time behind tests; any member that
early-returns on success is either refactored to fall through or excluded (logged,
not silently dropped).

## Layout / ABI Impact

None. Reuses `materialize_current_result` and the existing `Result OF T` register
form. The byte-identical golden suite guards untrapped call sites.

## Phases

### Phase 1 — Generic raw wrapper + dispatch

Stand up the mechanism with no members enabled yet (still gated), so the wrapper
can be unit-exercised before flipping the gate.

- [ ] Add `lower_inline_builtin_raw(&mut self, target, args)` to
      `src/target/shared/code/builder_values.rs`, factoring the capture / Ok-tag /
      materialize scaffold out of `lower_inline_conversion_raw` and dispatching to
      the member `lower_*` by `native_builtin_target(target)`. Have
      `lower_inline_conversion_raw` delegate to it (or share the scaffold) so there
      is one raw path.
- [ ] Route the inline-TRAP raw call site (`builder_values.rs:666-694`) to the new
      wrapper for members once they are enabled; keep the backstop for anything
      still in `inline_trap_unsupported`.

Acceptance: `cargo build` clean; the conversion built-ins still lower through the
shared scaffold and their existing `_valid` trap goldens are byte-identical
(refactor-only, no behavior change).
Commit: —

### Phase 2 — Enable fallible members + tests

Flip the gate per member, verifying runtime trap behavior for each.

- [ ] Remove the census's fallible members from `inline_trap_unsupported`
      (`src/builtins/mod.rs:170-174`) — narrow `native_builtin_target(...).is_some()`
      to the infallible remainder (or subtract the fallible set explicitly).
- [ ] For each enabled member, confirm its `lower_*` success path falls through
      cleanly (no early `return_()`); refactor or exclude-and-log otherwise.
- [ ] Tests: `tests/func_collections_get_valid/**` (and the other enabled
      members) gain an inline-TRAP-with-RECOVER program whose runtime output shows
      the recovered value on the failing index — an execution proof, not just a
      golden. Add a value-less/`Nothing` case if any enabled member is value-less.

Acceptance: a program inline-TRAPping `collections::get(xs, i)` out-of-range
RECOVERs and prints the default at runtime (observed, not just compiled); the same
for each other enabled member.
Commit: —

### Phase 3 — Backstop, docs, acceptance

- [ ] Confirm `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` is no longer reachable from
      the front-end for any enabled member and the codegen backstop still fires for
      the infallible set (a regression guard test that a `bits::*` trap still errors).
- [ ] Update `src/docs/spec/language/error-model.md` §8.6 rule 14 to state the
      fallible inline members now support inline TRAP (only infallible inline
      built-ins remain rejected, via rule 11).
- [ ] Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`
      passes; native-code goldens for untrapped sites byte-identical.

Acceptance: full acceptance green and the error-model spec reflects the enabled
members.
Commit: —

## Validation Plan

- Function tests: `_valid` inline-TRAP runtime proofs for every enabled member;
  `_invalid` regression that an infallible built-in trap still errors.
- Runtime proof: out-of-range `collections::get` inline TRAP RECOVERs at runtime.
- Doc sync: error-model §8.6 rule 14.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **One wrapper vs. keep conversions separate** — recommend **one** shared
  scaffold (`lower_inline_conversion_raw` delegates to `lower_inline_builtin_raw`)
  so the raw path can't drift from the normal dispatch. Alternative: leave
  conversions untouched and add a parallel wrapper — rejected as duplicative.

## Summary

The risk is entirely the success-fall-through across heterogeneous member result
shapes; the error paths already honor the capture, so no per-member error work is
needed. Members are enabled one at a time behind runtime-proof tests, and the
byte-identical golden suite guards that untrapped call sites are untouched.

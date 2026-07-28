# MFBASIC Inline TRAP on Inline-Lowered Built-ins Plan

Last updated: 2026-07-03
Overall Effort: large (3h–1d)

This is the umbrella overview for `plan-21`. The feature makes the inline
`TRAP(e)` form behave sensibly for the built-ins that are compiled inline
(spliced at the call site with no callable symbol). Today attaching an inline
TRAP to any inline-lowered built-in is a hard compile error
(`TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`), regardless of whether the built-in can
actually fail. That is confusing on two fronts:

1. **Infallible built-ins** (`len`, `toString`, `typeName`, `bits::*`, and the
   pure-query members) get an error whose remedy — "move it into a FUNC/SUB and
   TRAP there" — implies you could then catch something, but wrapping `len` traps
   nothing. The accurate message is "this call cannot fail."
2. **Fallible inline members** (`collections::get` out-of-range,
   `strings::mid` range, `collections::set`/`insert`/`removeAt`, …) genuinely
   fail at runtime, yet you cannot inline-TRAP them even though the sibling
   conversion built-ins (`toInt`, `toFloat`, …) — which are *also* inline-lowered
   — support inline TRAP fine.

The whole feature splits by effort into two independently-landable sub-plans:

- **[plan-21-A](plan-21-A-inline-trap-diagnostics.md)** — small. Classify the
  inline-lowered built-ins by fallibility and route the diagnostics accordingly:
  infallible built-ins hit the existing accurate `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`
  ("this expression cannot fail"); `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` narrows
  to the still-unsupported *fallible* inline members. Pure diagnostics; no new
  capability. This lands the fallibility census that B consumes.
- **[plan-21-B](plan-21-B-inline-trap-raw-lowering.md)** — medium. Give the
  fallible inline members a raw-`Result` inline lowering (generalizing the
  existing `lower_inline_conversion_raw`) and drop them from
  `inline_trap_unsupported`, so `collections::get(xs, i) TRAP(e)` compiles and
  runs. After B, `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` is no longer reachable
  from the front-end and survives only as the codegen backstop.

B depends on A's fallibility census but is otherwise independent; A is valuable
on its own (it removes the confusion even if B never ships).

It complements:

- `./mfb spec language error-model` (§8.4 inline TRAP, §8.6 rules 11 & 14; the
  canonical specs live under `src/docs/spec/language/error-model.md`)
- `./mfb spec diagnostics error-codes` (`TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`
  `2-203-0102`, `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`; build input for
  `errorCode::`)

## 1. Goal

- An inline TRAP on an infallible built-in reports "this call cannot fail"
  (`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`), not the misleading "move it into a
  FUNC/SUB" message.
- An inline TRAP on a fallible inline member (`collections::get`, `strings::mid`,
  …) compiles and traps the real runtime error, matching the behavior of `toInt`
  and helper-backed built-ins.

### Non-goals (explicit constraints)

- **No language-surface change.** No new keywords, no change to inline-TRAP
  placement rules (§8.6 rule 11: still only on LET/MUT/assignment/bare-expr), no
  change to the `RECOVER`/diverge handler contract (rule 12).
- **No value/copy/move/drop change.** The trapped-value ownership and lexical
  drop on the error path are unchanged; the raw path materializes a `Result OF T`
  exactly as the conversion built-ins already do.
- **No layout/ABI change.** The fallible-call register ABI (`mfb spec memory
  fallible-call-abi`) and every golden binary for untrapped call sites stay
  byte-identical — the raw wrapper only fires when a TRAP is attached.
- **Hot-path untouched.** `len`, `get`, `mid` stay spliced-inline at their
  ordinary (untrapped) call sites; the raw wrapper adds no cost there.

## 2. Current State

- **Front-end gate** — `src/typecheck/inference.rs:90-142`
  (`Expression::Trapped`). `fallible` is computed as
  `!builtins::is_package_constant(canonical)` (line 102-105): every non-constant
  call is treated as fallible, so `len` is "fallible" and reaches the inline-built-in
  check at line 129-141, which reports `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`. The
  accurate `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` (line 115-122) fires only for
  package constants today.
- **The unsupported set** — `src/builtins/mod.rs:170-174`
  (`inline_trap_unsupported`): `bits::is_bits_call(target) ||
  native_builtin_target(target).is_some() || matches!(target, "len" | "toString"
  | "typeName")`. `native_builtin_target` (`src/builtins/mod.rs:114-145`)
  enumerates the dequalified `collections.`/`strings.` members.
- **The working precedent** — `src/target/shared/code/builder_values.rs:1226-1259`
  (`lower_inline_conversion_raw`): sets `self.raw_result_capture = Some(capture)`,
  runs the built-in's normal inline lowering, then on the success fall-through
  tags the value `Ok` and lands at the capture label to materialize a `Result OF
  T`. `toInt`/`toFloat`/`toFixed`/`toByte` route through this for inline TRAP
  (`builder_values.rs:666-694`).
- **The generic hinge (key finding)** —
  `src/target/shared/code/builder_codegen_primitives.rs:667`
  (`emit_error_register_return`): *every* inline built-in's error exit already
  branches to `raw_result_capture` instead of returning when a capture is active.
  Every fallible inline member raises through `emit_error_code_return`
  (`builder_codegen_primitives.rs:301`, e.g. `ERR_INDEX_OUT_OF_RANGE_CODE`
  `77050001`, `ERR_NOT_FOUND_CODE` `77050004`). So the raw lowering is **not**
  per-builtin work — the error paths already cooperate; only the success
  fall-through + dispatch needs generalizing.
- **The inline member dispatch** — `src/target/shared/code/builder_values.rs:516-593`
  maps each `native_builtin_target` result to its `lower_*` method
  (`lower_collection_get` `builder_collection_queries.rs:25`, `lower_mid`
  `builder_search.rs:473`, `lower_collection_set`, `lower_collection_insert`,
  `lower_collection_remove_at`, `lower_replace` `builder_strings.rs:4`, …).

## 3. Design Overview

Two layers, matching the sub-plans:

- **A (front-end):** add a fallibility classifier for built-in callees. The
  inline-TRAP gate uses it to pick the right diagnostic. This requires a census —
  which inline members can raise a *domain* error a program would want to trap
  (index-out-of-range, not-found, range/format) versus which are pure queries or
  can only OOM. The census is the shared artifact both sub-plans key on.
- **B (codegen):** generalize `lower_inline_conversion_raw` into a member-agnostic
  `lower_inline_builtin_raw(target, args)` that dispatches to the member's normal
  `lower_*` method under a raw capture, tags success `Ok`, and materializes.
  Remove the fallible members from `inline_trap_unsupported` so the gate lets them
  through and codegen routes them to the raw wrapper.

Correctness risk concentrates in B: some member lowerings return non-scalar or
`Nothing`-typed success values (mutating ops return the updated collection;
`forEach` returns nothing), and the success fall-through must handle each without
assuming a single value register. The census in A determines which members are
in scope precisely so B never wraps a member that has an early success `return_()`
or an unusual result shape it can't materialize.

## Layout / ABI Impact

None. No change to `mfb spec memory` / `mfb spec package`. The raw wrapper reuses
the existing `Result OF T` materialization (`materialize_current_result`,
`builder_arena_transfer.rs:78`) that conversion built-ins and helper-backed calls
already emit. Untrapped call sites are byte-identical; the byte-identical golden
suite is the oracle.

## Phases

See the sub-plans for the concrete, ordered task lists:

- **plan-21-A** — Phase 1 (fallibility census + classifier), Phase 2 (diagnostic
  routing + spec/diagnostics sync + tests).
- **plan-21-B** — Phase 1 (generic raw wrapper + dispatch), Phase 2 (per-member
  enablement + tests), Phase 3 (backstop + acceptance).

## Validation Plan

- Function tests: `tests/func_<pkg>_<func>_valid/**` and `_invalid/**` per
  sub-plan (A adds `_invalid` diagnostic goldens; B adds `_valid` trap-runtime
  proofs).
- Runtime proof (B): a program that inline-TRAPs `collections::get(xs, i)` with
  an out-of-range `i`, RECOVERs a default, and prints it — observing the default,
  not a process abort.
- Doc sync: `src/docs/spec/language/error-model.md` §8.6 rules 11 & 14;
  `src/docs/spec/diagnostics/**` error-codes table.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **OOM as fallible-for-TRAP** — recommend **no**: an inline member that can only
  fail via allocation OOM (`append`/`prepend`/`insert` growth) stays classified
  infallible-for-TRAP, consistent with how inline allocation failure propagates
  everywhere else. Only domain errors (index-out-of-range, not-found, range,
  invalid-format) make a member trappable. Alternative: treat OOM as trappable for
  uniformity — rejected as surprising and untestable. (Finalized by A Phase 1.)
- **`getOr`/`find` classification** — `getOr` has a caller-supplied default and
  never raises → infallible. `find` on a negative start raises `77050001`
  (`builder_search.rs:158`) → fallible. Census confirms per-member. (A Phase 1.)

## Summary

The engineering risk is entirely in B's success-fall-through generalization
across heterogeneous member result shapes; A is a low-risk diagnostic reroute
that also produces the census B depends on. Nothing in the layout, ABI, or
untrapped-call codegen changes — the byte-identical golden suite guards that.

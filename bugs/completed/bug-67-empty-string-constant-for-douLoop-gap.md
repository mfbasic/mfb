# bug-67: `op_requires_empty_string_constant` skips `FOR` and `DO…LOOP UNTIL` bodies, so an uninitialized String declared inside a `FOR`/`DO` loop fails the build with a dangling `_mfb_str_empty` relocation

Last updated: 2026-07-09
Effort: small (<1h)

The analysis that decides whether the module needs the shared empty-string data object
(`_mfb_str_empty`) walks the NIR ops with `op_requires_empty_string_constant`, but its
recursion descends only into `If`/`Match`/`While`/`ForEach`/`Trap` bodies — `NirOp::For`
and `NirOp::DoUntil` fall through to `_ => false`. So an uninitialized String local (or a
record with a String field) declared **inside a `FOR` or `DO…LOOP UNTIL` body**, when
nothing else in the module already forces the constant, is missed: the emitter still lowers
the uninitialized bind to a `data` relocation against `_mfb_str_empty`, but the data object
is never emitted, and the native plan fails validation.

Runtime-confirmed: `FOR i = 0 TO 2 : MUT s AS String : io::print(s) : NEXT` fails to build
with `error: native code data relocation target '_mfb_str_empty' is not a data object or
defined symbol`; the equivalent `WHILE` version builds and runs. This is a **valid program
that fails to compile** — the same incomplete-loop-traversal class as bug-45, and a
re-introduction of bug-05's dangling `_mfb_str_empty`. The single correct behavior a fix
produces: an uninitialized String inside any loop form compiles exactly as it does at top
level or inside `WHILE`.

References:

- `src/target/shared/code/module_analysis.rs:op_requires_empty_string_constant` (`:13-40`):
  loop arm `While | ForEach | Trap` (`:35-37`), catch-all `_ => false` (`:38`) — no `For`,
  no `DoUntil`. Driver `module_requires_empty_string_constant` (`:3`) gates the
  `EMPTY_STRING_SYMBOL` emission at `mod.rs:447`.
- Emitter that unconditionally references the constant: `builder_control.rs:156` →
  `lower_default_value` → `builder_value_semantics.rs:67-74` (String) / `:90-98` (record
  String fields) → `load_empty_string_constant` (`builder_emit_helpers.rs:86`) →
  `emit_load_static_string_symbol` (`:129-143`, pushes the `_mfb_str_empty` data reloc).
- Rejection: `NativeCodePlan::validate` (`validation.rs:158-166`).
- The type-side check `type_requires_empty_string_constant` is correct (String + record
  String fields); only the **op traversal** is wrong.
- Same class: bug-45 (`validate.rs:collect_bind_types` skips `ForEach`). Re-intro of bug-05
  (dangling `_mfb_str_empty`), see `src/target/shared/nir/lower.rs:161`.
- Systemic note: the sibling traversals in the *same file*
  (`ops_bind_type_in`, `ops_may_record_cleanup_failure`, `ops_use_call`,
  `ops_use_type_name`, `ops_may_emit_float_arithmetic_error`, `ops_use_unicode_runtime_tables`)
  all *do* handle `For`/`DoUntil` — this function is the lone straggler.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

```
IMPORT io
SUB main()
  FOR i = 0 TO 2
    MUT s AS String
    io::print(s)
  NEXT
END SUB
```

- Observed: `error: native code data relocation target '_mfb_str_empty' is not a data
  object or defined symbol`.
- Expected: builds and prints three empty lines.

Contrast cases that work correctly today (verified):

- The same uninitialized String bind inside `WHILE … WEND` → builds.
- The same bind at function top level → builds.
- `DO … LOOP UNTIL` also fails (the other missing arm).

## Root Cause

`op_requires_empty_string_constant` omits `NirOp::For` and `NirOp::DoUntil` from its
recursion, so a String bind nested only in those loop forms does not set the "module needs
`_mfb_str_empty`" flag, and `mod.rs:447` skips emitting the data object — while the codegen
path for the uninitialized bind emits the relocation regardless. The plan validator then
rejects the dangling reference.

## Goal

- An uninitialized String (or String-bearing record) inside a `FOR`/`DO…LOOP UNTIL` body
  triggers emission of `_mfb_str_empty`; the reproduction builds and runs.
- All loop forms are handled identically for this analysis.

### Non-goals (must NOT change)

- The type-side check and the codegen emission of the relocation.
- Programs that already build (no spurious extra constant beyond correctness).

## Blast Radius

- `op_requires_empty_string_constant` (`module_analysis.rs`) — fixed here.
- **Systemic:** audit every remaining NIR/AST traversal in the codebase for missing
  `For`/`DoUntil`/`ForEach` arms — bug-45 (`validate.rs`) and this are two instances of the
  same class; there may be more. Prefer exhaustive matches over `_ => false`.

## Fix Design

Extend the loop arm to include the missing variants:
`NirOp::While { body, .. } | NirOp::For { body, .. } | NirOp::DoUntil { body, .. } |
NirOp::ForEach { body, .. } | NirOp::Trap { body, .. } => body.iter().any(...)`. Make the
match **exhaustive** over `NirOp` (drop the `_ => false`) so a future variant cannot
silently regress — matching the neighboring functions in the file.

## Phases

### Phase 1 — failing test

- [x] Add build tests: uninitialized String inside `FOR` and inside `DO…LOOP UNTIL` must
      build (fail today). Add the `WHILE`/top-level cases as guards.
- [x] Grep all NIR/AST traversals for `_ => false`/`_ => {}` catch-alls that could hide a
      missing loop arm; list them for a follow-up audit.

### Phase 2 — the fix

- [x] Add the `For`/`DoUntil` arms (make the match exhaustive).

### Phase 3 — validation

- [x] `scripts/test-accept.sh`; confirm the reproduction builds and prints three empty
      lines; no other program's build status changes. (Orchestrator runs test-accept.sh;
      reproduction + targeted tests verified here.)

## Validation Plan

- Regression test(s): uninitialized-String-in-`FOR`/`DO` build tests + `WHILE`/top-level
  guards.
- Runtime proof: build and run the reproduction.
- Doc sync: none expected.
- Full suite: `scripts/test-accept.sh`.

## Summary

One analysis function omits `FOR`/`DO…LOOP UNTIL` from its loop-body recursion, so an
uninitialized String in those loops references an empty-string data object the module never
emits and the build fails — a valid program rejected, re-introducing bug-05's class. The fix
is the two missing match arms (made exhaustive); the broader lesson is to audit every
loop-body traversal for the same gap that also produced bug-45.

## Resolution

Fixed in `src/target/shared/code/module_analysis.rs`:

- `op_requires_empty_string_constant` now recurses into `NirOp::For` and `NirOp::DoUntil`
  bodies (added to the `While | ForEach | Trap` arm) and the match was made **exhaustive**
  over `NirOp` — the `_ => false` catch-all was replaced by an explicit list of the
  non-recursive ops (`Bind { value: Some(_) }`, `StoreGlobal`, `Assign`, `StateAssign`,
  `Return`, `ExitLoop`, `ContinueLoop`, `ExitProgram`, `Fail`, `Eval`). A future
  body-bearing variant now forces a compile error rather than a silent miss.
- Hardening: the sibling `ops_bind_type_in` traversal (already handling all five loop
  bodies) had its `_ => false` catch-all converted to the same exhaustive list, closing
  the future-variant hole there too.

Audit result: the remaining traversals in this file
(`ops_may_record_cleanup_failure`, `ops_may_emit_float_arithmetic_error`, `ops_use_call`,
`ops_use_type_name`, `ops_use_unicode_runtime_tables`) were already exhaustive and handled
every loop form. A tree-wide grep of the other NIR traversals
(`plan/symbols.rs`, `code/data_objects.rs`, `nir/json.rs`, `plan/function_builder.rs`)
confirmed each explicitly handles `For`/`DoUntil`/`ForEach`; their catch-alls cover only
non-loop ops. `validate.rs` (`collect_bind_types` / bug-45) is owned by another agent and
was not touched.

Regression tests added to `tests/native_loop_runtime.rs`:
`native_uninitialized_string_in_for_loop_builds`,
`native_uninitialized_string_in_do_until_loop_builds`, and the guard
`native_uninitialized_string_in_while_loop_builds`. Before the fix the `FOR`/`DO` tests
failed with `error: native code data relocation target '_mfb_str_empty' is not a data
object or defined symbol`; after the fix all pass. Runtime reproduction built and ran,
printing three empty lines for the `FOR` loop and three for the `DO … LOOP UNTIL` loop.

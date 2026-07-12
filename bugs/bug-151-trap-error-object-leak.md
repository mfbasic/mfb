# bug-151 — Caught `Error` object leaks on every taken `TRAP` (trap-in-a-loop grows RSS)

Discovered 2026-07-11 while validating bug-147.5(a) (the collections `set`
error-path leak). The set-specific leak turned out to be dwarfed by a **general**
leak in the error-handling path that has nothing to do with collections.

## Symptom

Every time a `TRAP(e)` handler actually catches an error, the caught `Error`
object is leaked into the arena and never freed. In a loop that traps each
iteration — a retry loop, per-item error handling, a validation loop — RSS grows
linearly at ~0.6 KB per catch.

Measured (host aarch64, `/usr/bin/time -l` max RSS):

| scenario (200,000 iterations)            | max RSS |
|------------------------------------------|---------|
| `TRAP(e)` present but **never taken**    | ~1.0 MB (flat) |
| bare `FAIL error(...)` caught by `TRAP`  | ~118 MB |
| `collections::get` OOB caught by `TRAP`  | ~132 MB |
| `collections::set` OOB caught by `TRAP`  | ~132 MB |

The not-taken baseline is flat; the taken cases grow ~linearly with the iteration
count. `get`-OOB (no intermediates at all) leaks the same as `set`-OOB, proving the
leak is the caught `Error` block itself, not any collection intermediate.

## Root cause

The trap error binding `e` is allocated a function-level stack slot in
`function_lowering.rs:688` and re-pinned to that slot at the handler entry in
`builder_control.rs:772`, but it is **never registered as an
`ActiveCleanup::OwnedValue`**. The routed `Error` is a fresh arena block (built by
`_mfb_make_error_result` / `emit_build_error_inline`) whose pointer is stored into
the trap slot; nothing frees that block when the handler exits. Every catch builds
a new block, so every catch leaks one.

## Fix (not yet applied — double-free-prone, needs its own validated change)

Register `e` as an `ActiveCleanup::OwnedValue` at the trap slot when the handler
body begins, so every exit from the handler frees it exactly once. This is the same
double-free-prone class as the "TRAP cleanup double-free" history — the caught error
can *escape* the handler and must then NOT be freed:

- `RETURN e` (handler in an `Error`-returning FUNC) — already covered by the
  existing `plan_returned_move` move-elision (it drops the owned-value cleanup for a
  returned owned local).
- `FAIL e` (re-raise) — routes through `emit_error_value_exit`, which for the
  in-trap-body case frees all `active_cleanups` (including `e`) and *then* propagates
  `e`'s registers → freeing the very block being propagated (use-after-free). Needs
  a `plan_returned_move`-equivalent for the `Fail` path: when the failed value is the
  trap local, drop its cleanup for that exit.
- `FAIL <other>` / implicit propagation of a *different* error — `e` is freed, the
  other error propagated; safe as long as the other error is a distinct block (it is;
  building a new error from `e`'s fields deep-copies them).

Validation before this can land: a trap-in-a-loop RSS-flat test, plus double-free
tests for `FAIL e`, `RETURN e`, nested `TRAP`, `e`-field access, and `e` captured
into an escaping value — on all four remotes.

## Scope

Affects every target (shared codegen). Not remote-specific. Correctness-critical for
long-running programs that handle errors in a loop.

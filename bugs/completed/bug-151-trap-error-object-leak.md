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

## Fix — APPLIED (2026-07-11)

Register `e` as the FIRST `ActiveCleanup::OwnedValue` of the handler body's own
cleanup scope (mirroring `lower_ops`: push `cleanup_scope_starts`, register `e`,
then `lower_ops_inner`). The body's existing scope-drop then frees `e` exactly once
on every handler exit — RETURN, FAIL, or fall-through — and NEVER on the success
path that branches over the handler (where the slot is never written; the free is
null-guarded regardless). The escape cases are handled by existing machinery:

- `RETURN e` (handler in an `Error`-returning FUNC) — already covered by the
  existing `plan_returned_move` move-elision (it drops the owned-value cleanup for a
  returned owned local).
- `FAIL e` (re-raise) — safe without new move logic: `emit_error_value_exit` calls
  `store_pending_error_from_value` → `lower_value_owned(e)` FIRST, which deep-copies
  the error into a standalone block before the scope-drop frees the original `e`, so
  the propagated copy is not use-after-freed. (The deep copy itself is then orphaned —
  a separate pre-existing leak, filed as bug-152 — but there is no double-free.)
- `FAIL <other>` / implicit propagation of a *different* error — `e` is freed, the
  other error propagated; safe (the other error is a distinct block; building a new
  error from `e`'s fields deep-copies them).

Validated (host + all four remotes: Kali aarch64, Alpine x86_64 musl, Ubuntu x86_64
glibc, Alpine riscv64 musl):
- bare `FAIL … TRAP(e){RETURN}` in a 200 K loop — RSS flat (~20 MB at 100 K/200 K/
  400 K); was ~118 MB and growing.
- `RETURN e` from an `Error`-returning FUNC — correct error returned, no double-free.
- `FAIL e` re-raise through two levels — correct propagation, no double-free/UAF.
- `e.code`/`e.message` read then a derived value returned — fields intact.
- 40+ existing trap tests crash-free; the flagged error/syntax tests still match
  their committed goldens; no golden churn (no trap test carries codegen goldens).

Regression: `tests/rt-behavior/trap/bug151_caught_error_freed`.

## Residual

The `FAIL e` re-raise path still leaks the deep-copied propagation transient
(bug-152) — a pre-existing error-Result-ABI issue, not a regression here. The common
catch-and-handle path (the vast majority of trap usage) is fully fixed.

## Scope

Affects every target (shared codegen). Not remote-specific. Correctness-critical for
long-running programs that handle errors in a loop.

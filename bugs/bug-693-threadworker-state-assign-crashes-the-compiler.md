# bug-693: `RES worker AS ThreadWorker … STATE W` plus a write to `worker.state` crashes the compiler

Last updated: 2026-09-24
Effort: medium (1h–2h) — fixed as part of plan-156
Severity: MEDIUM
Class: Correctness (internal compiler error)

Status: Open — to be fixed by plan-156 (per-view STATE on Thread and ThreadWorker)
Regression Test: none yet — `tests/rt-behavior/threads/bug693-worker-state/` (plan-156-E Phase 1)

Declaring a `STATE` on a `ThreadWorker` parameter and assigning to it aborts
the build with an internal error instead of either compiling or giving a
diagnostic:

```
error: native code WITH target 'Unknown' is not a record while lowering state assign worker
```

The cause is the grammar: `STATE W` after a thread type attaches to the
thread's *output* type (`ParameterType::with_state` puts it on `out`), so the
type is a `ThreadWorker` whose output is `Integer STATE W`. The state
assignment then finds no record to write to. So a thread `STATE` clause is
accepted by the parser but means something nobody wrote.

**Correct behavior after the fix.** Under plan-156, `STATE W` on a
`ThreadWorker` declares the worker view's own STATE, and `worker.state.n = 3`
writes it. Any spelling the compiler cannot lower is a diagnostic, never an
internal error.

References:

- Spec §15 (STATE), §16 (thread planes). plan-156 (per-view STATE; the parse
  rule for `Thread OF M TO O STATE P`).
- Found by the plan-156 design review, 2026-09-24.

## Failing Reproduction

```basic
IMPORT io
IMPORT thread
TYPE W
  n AS Integer
END TYPE
ISOLATED FUNC work(RES worker AS ThreadWorker OF Integer TO Integer STATE W, n AS Integer) AS Integer
  worker.state.n = 3
  RETURN n
END FUNC
FUNC main AS Integer
  LET t = thread::start(work, 5)
  io::print(toString(thread::waitFor(t)))
  RETURN 0
END FUNC
```

- Observed: `mfb build` prints the internal error above and writes no
  executable.
- Expected (plan-156): builds; prints `5`.

Contrast: without the `worker.state` write the program builds. Without `RES`,
`worker AS ThreadWorker … STATE W` is `MFB_PARSE_UNEXPECTED_TOKEN`, because
the parser reads a parameter's STATE only after `RES`
(`src/ast/items.rs:parse_params` → `parse_optional_state`).

## Root Cause

- `src/types.rs:ParameterType::with_state` on a `ThreadHandle` attaches STATE
  to `out` (the type becomes `Thread… TO (O STATE W)`), not to the handle.
- The native lowering of a state assignment then resolves `worker`'s STATE
  type to `Unknown` and aborts with the message above instead of reporting a
  diagnostic.

## Goal

- Under plan-156: the reproduction builds and prints 5. A thread STATE
  spelling that is still unsupported is a rule-coded diagnostic, not an
  internal error.

### Non-goals

- `STATE` on ordinary resources is unchanged.

## Blast Radius

- `Thread OF … TO O STATE P` (parent side) has the same parse and is fixed by
  the same plan-156 parse rule.
- Other internal-error paths in state-assignment lowering: not audited here.
  plan-156's STATE phase adds a diagnostic for any unresolved STATE type.

## Fix Design

plan-156's STATE phase: a thread type's trailing `STATE` binds to the handle
(the view), and each view's STATE lives in its own view record.

## Phases

Tracked in plan-156-E (`planning/plan-156-E-*.md`); this document
records the reproduction and closes when that phase lands. plan-156-F Phase 3
archives it with a `STATUS: FIXED` block.

Commit: —

## Summary

An internal compiler error on a spelling the grammar half-accepts. Fixed by
plan-156.

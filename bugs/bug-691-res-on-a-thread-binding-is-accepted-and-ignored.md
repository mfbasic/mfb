# bug-691: `RES` on a `Thread` binding or parameter compiles and is ignored — a `RES` thread parameter still takes the caller's handle and closes it

Last updated: 2026-09-24
Effort: large (3h–1d) — fixed as part of plan-156
Severity: MEDIUM
Class: Footgun

Status: Open — to be fixed by plan-156 (Thread and ThreadWorker become RES resources)
Regression Test: none yet — `tests/rt-behavior/threads/bug691-res-thread-param/` (plan-156-C Phase 1)

`RES` is the ownership keyword for resources (spec §15). On anything that is
not a resource the verifier should reject it (`TYPE_RES_REQUIRES_RESOURCE`).
On a `Thread` it does neither: `RES t AS Thread OF … = thread::start(...)`
compiles and behaves exactly like `LET`, and a parameter declared
`RES t AS Thread OF …` still *moves* the handle into the callee. The callee
closes it when it returns, the opposite of what `RES` promises (§15: "Passing a
`RES` binding to an ordinary function hands the callee that same pointer … the
caller's binding stays live after the call"). The program compiles cleanly and
fails at runtime with `ErrResourceClosed`.

**Correct behavior after the fix.** Today (a thread is not a resource), `RES`
on a thread must be a compile error. Under plan-156 (a thread *is* a
resource), `RES` becomes the only legal spelling, and a `RES` thread parameter
borrows exactly like any other resource parameter. Either way, a program must
never compile `RES` and then get `LET` semantics.

References:

- Spec §15 (`src/docs/spec/language/15_resource-management.md`): RES semantics
  and parameter passing. §16 (`16_threads.md`): Thread handle ownership.
- plan-156 (the fix). bug-692 (the record-field hole in the same checks).
- Found by the plan-156 design review, 2026-09-24.

## Failing Reproduction

```basic
IMPORT io
IMPORT thread

ISOLATED FUNC work(worker AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC peek(RES t AS Thread OF Integer TO Integer) AS Boolean
  RETURN thread::isRunning(t)
END FUNC

FUNC main AS Integer
  RES t AS Thread OF Integer TO Integer = thread::start(work, 5)
  io::print(toString(peek(t)))
  io::print(toString(thread::waitFor(t)))
  RETURN 0
END FUNC
```

```
mfb build <dir> && ./build/<name>.out
```

- Observed: builds with no diagnostic; prints `TRUE`, then
  `Error: 7-703-0004 Resource handle is already closed.` (exit 255).
- Expected (today's model): a compile error on both `RES` uses. Expected
  (plan-156): `TRUE` then `5`, exit 0.

Contrast: the same shape with `RES f AS fs::File` and `peek(RES f AS fs::File)`
keeps `f` open after the call (measured 2026-09-24: reading three lines through
two calls and the caller printed `one`, `two`, `three`, exit 0). A
`RES t AS Thread` binding with no parameter pass (just `waitFor`) runs and
prints 5, which is why the hole went unnoticed.

## Root Cause

- `src/ir/verify/ops.rs` (the `Bind` arm): `TYPE_RESOURCE_REQUIRES_RES` fires
  only when the type is a resource, and `TYPE_RES_REQUIRES_RESOURCE` only when
  `src/ir/verify/link.rs:provably_data_type` is true. For
  `ParameterType::ThreadHandle`, `is_resource` is false *and*
  `provably_data_type` is false, so neither rule fires.
- `src/ir/verify/resources.rs:res_axis_slot` has the same gap for `RES`
  fields.
- Parameters are never checked at all: the verifier's RES check runs only on
  `IrOp::Bind`. The parser records `resource = match_keyword(Res)`
  (`src/ast/items.rs:parse_params`), and only
  `src/ir/resource_escape.rs` reads it.
- Codegen then treats the parameter as a thread, not as a RES pointer:
  `src/codegen/engine/function/function_lowering.rs:lower_function` pushes a
  thread cleanup plus an owner increment (bug-622), and
  `src/codegen/cleanup/thread/builder_thread_cleanup.rs:deactivate_moved_thread_arguments`
  moves the caller's handle into the call.

## Goal

- Under plan-156: the reproduction prints `TRUE` then `5`, exit 0, and a
  `LET`/`MUT` thread binding is rejected.

### Non-goals

- Do not patch this separately by rejecting `RES` on threads: plan-156 makes
  `RES` the required spelling, so an interim rejection would be reversed within
  the same change series. The fix is plan-156's verifier phase.

## Blast Radius

- `ops.rs` Bind arm and `res_axis_slot`: fixed by plan-156 (a ThreadHandle
  becomes a resource in both predicates).
- RES parameters of any non-resource type: not checked by the verifier either
  (params skip the Bind check). That's latent for other types, and out of
  scope for this bug; plan-156's verifier phase audits it.

## Fix Design

Fixed by plan-156, verifier phase: `ThreadHandle` joins `is_resource_type` /
`close_op_for` with a hidden drop op, and the thread-specific move-on-pass
codegen is deleted. The regression test is the reproduction above.

## Phases

Tracked in plan-156-C (`planning/plan-156-C-*.md`); this document
records the reproduction and closes when that phase lands. plan-156-F Phase 3
archives it with a `STATUS: FIXED` block.

Commit: —

## Summary

A silent footgun: `RES` looks like the safe spelling and behaves like the
unsafe one. The fix is plan-156, not a standalone patch.

# bug-975: plan-156 left the worker view out of the resource rules — `ThreadWorker` binds with `LET`/`MUT`, because resource-ness is keyed on having a close op

Last updated: 2026-10-05
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (incomplete plan-156; owner-confirmed)

Status: Open
Regression Test: tests/syntax/threads/threadworker-let-binding-invalid and threadworker-mut-binding-invalid (TYPE_RESOURCE_REQUIRES_RES); tests/rt-behavior/threads/threadworker-res-binding-rt (to add)

**Owner decision (2026-10-05):** a `ThreadWorker` is a Resource, like the parent `Thread`.
It is bound only with `RES`. `LET t2 = t` and `MUT t3 AS ThreadWorker OF … = t` do not
compile (`TYPE_RESOURCE_REQUIRES_RES`). Split out of bug-857 (§14 item 1), where the
spec's "non-copyable values move" claim contradicted the observed aliasing.

## This is a plan-156 gap, not a new language change

plan-156 already settled that both views are resources on the same thread, from opposite
sides (`planning/completed/plan-156-A-thread-table.md`, §The plan-156 feature):

- "**Two views, two resources.** `Thread` (the parent view) and `ThreadWorker` (the worker
  view) are each a standard resource record in their holder's arena … plus a pointer to the
  control block."
- "**No user-visible close.** Only the runtime closes the worker view, when the worker
  function returns."
- "**Clean break.** `LET`/`MUT` bindings of a thread become a compile error; `RES` is the
  only spelling."

plan-156-C (commit series after `9f49cf2a8`) made both views `ParameterType::ThreadHandle
{ worker, .. }` resource records. But its Phase 2 note says "`close_op_for` answers
`thread.drop` … for a parent view and `None` for a worker view", and every resource
predicate answers through `close_op_for`:

- `src/ir/verify/link.rs:is_resource_or_resource_union` returns true only if
  `close_op_for(base).is_some()`.
- `src/ir/verify/resources.rs:is_resource_type` takes the same `close_op_for` path.
- `src/ir/verify/ops.rs` (Bind arm) emits `TYPE_RESOURCE_REQUIRES_RES` only when
  `is_resource_or_resource_union` holds.

So the worker view, which deliberately has no close op because the runtime closes it at
worker return, is not a resource to the verifier, and the plan-156-F clean break never
reached it. plan-156-F's migration and its rejection fixture
(`tests/syntax/threads/let_thread_binding_invalid/`) cover only `= thread::start` (parent)
bindings. Its census regex was `^(\s*)(LET|MUT)\s+(\w+)(\s+AS\s+Thread OF[^=]*)?\s*=\s*thread::start`.

**Owner rule (2026-10-05): no special cases.** `Thread` and `ThreadWorker` are 100% normal
resources. The only new thing is that the `ThreadWorker` resource has no close. So the fix
is not a `ThreadHandle { worker: true }` arm. It removes the coupling: "is a resource" must
not be decided by "has a close op". A resource type with no close is still a resource for
every predicate (`is_resource_or_resource_union`, `is_resource_type`, the `RES`-binding rule,
moves, aliasing, storage), and its scope cleanup simply emits no close. Any
`ThreadHandle`-specific branches that only exist to compensate (e.g. plan-156-E's
`thread_view` STATE carve-out in `parse_params`) are removed rather than extended.

**Parameter spelling:** parameters no longer take `RES` at all. A parameter is a resource
because its type is one (owner decision, **bug-976**). So the worker entry is
`worker AS ThreadWorker OF … [STATE W]` with no keyword, like every other resource parameter.

## Verified today (HEAD 846aa55fd, `/tmp/hardening/mfb-165`, `/tmp/hardening/coord/tw`)

The compiler does not treat `ThreadWorker` as a resource. Resource-ness is "has a
close op" (`src/ir/verify/resources.rs:is_resource_type` → `close_op_for`), and
`ThreadWorker` has none. It is also not a record. It is a built-in nominal
(`src/types.rs`, `THREAD_WORKER_TYPE`, alongside `List`/`Map`/`Thread` as a generic base).
The spec agrees with the code: `language/16_threads.md` says "A `ThreadWorker` has no close
op: the runtime owns it for the worker's whole run", and the entry parameter is
`worker AS ThreadWorker OF Msg TO Out STATE W`, "where `RES` is optional".

In an `ISOLATED FUNC spin(t AS ThreadWorker OF String TO Integer, n AS Integer)` started by
`thread::start(spin, 21)`, every form below builds and the program prints `thread=42`:

| binding inside the worker | result |
| --- | --- |
| `LET t2 = t` | builds, runs |
| `RES t2 = t` | builds, runs |
| `MUT t3 AS ThreadWorker OF String TO Integer = t` | builds, runs |
| `LET t2 = t` with the parameter declared `RES t AS ThreadWorker …` | builds, runs |

bug-857 also measured that `LET t2 = t` aliases. Sends through both names arrive, and a
`STATE` write through `t2` is visible through `t`.

Contrast: a parent `Thread` is a resource. `LET`/`MUT` binding is
`TYPE_RESOURCE_REQUIRES_RES`, and its close op is the hidden `thread.drop`.

## Goal

- `ThreadWorker` is classified as a resource everywhere `Thread` is (`is_resource_type`,
  `is_builtin_resource_type` in `src/codegen/resource/mod.rs`, the shape checker's
  `is_resource_type`). `LET`/`MUT` bindings of it are `TYPE_RESOURCE_REQUIRES_RES`.
- The worker entry's first parameter is a resource parameter by its type, with no `RES`
  keyword (bug-976). Only `LET`/`MUT` *bindings* of a worker view change.
- It has no user or scope close. The runtime closes the worker view at worker return (plan-156-A). Scope cleanup of a `RES` alias never tears down the running worker. A
  `RES t2 = t` alias follows the same Resource/alias rules the owner set for bug-843
  (one Resource, `RES` bindings are aliases, scope raised, never closed twice).
- Existing rules stay: no collection may hold one (`TYPE_COLLECTION_OWNERSHIP_VIOLATION`),
  and it never crosses a thread (`TYPE_THREAD_WORKER_NOT_TRANSFERABLE`).
- Spec: `language/16_threads.md` ("has no close op", "`RES` is optional"), `14_memory-semantics.md`
  §14.1/§14.2, `15_resource-management.md` ("Thread handles are resources too"), and
  `mfb man thread` describe `ThreadWorker` as a resource.

### Non-goals (must NOT change)

- Worker lifetime, the queues, cancellation, and the `STATE` plane semantics.
- Forbidden wrong fix: documenting the current aliasing as intended.

## Blast Radius

- Every thread fixture, example and `mfb man` example that binds a worker with `LET`/`MUT`
  (grep `ThreadWorker` under tests/, examples/, src/codegen/builtins/thread/).
- bug-844 (stateful worker send/receive misrouted), the same `ThreadWorker` handle path.

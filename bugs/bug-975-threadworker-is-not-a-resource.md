# bug-975: `ThreadWorker` is not a Resource — `LET`/`MUT`/`RES` all bind it, and copies alias one handle

Last updated: 2026-10-05
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (language surface; owner-confirmed)

Status: Open
Regression Test: tests/syntax/threads/threadworker-let-binding-invalid and threadworker-mut-binding-invalid (TYPE_RESOURCE_REQUIRES_RES); tests/rt-behavior/threads/threadworker-res-binding-rt (to add)

**Owner decision (2026-10-05):** a `ThreadWorker` is a Resource, like the parent `Thread`.
It is bound only with `RES`. `LET t2 = t` and `MUT t3 AS ThreadWorker OF … = t` do not
compile (`TYPE_RESOURCE_REQUIRES_RES`). Split out of bug-857 (§14 item 1), where the
spec's "non-copyable values move" claim contradicted the observed aliasing.

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
- The worker entry's first parameter is a `RES` parameter. Decide with the owner at
  implementation time whether the `RES` keyword becomes *required* there (today it is
  optional; making it required changes every thread program, man example and fixture).
- Its close op is hidden and does not end the worker, since the runtime owns the worker for
  its whole run. Scope cleanup of a `RES` alias never tears down the running worker. A
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

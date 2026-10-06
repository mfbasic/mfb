# bug-975: plan-156 left the worker view out of the resource rules — `ThreadWorker` binds with `LET`/`MUT`, because resource-ness is keyed on having a close op

Last updated: 2026-10-06
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (incomplete plan-156; owner-confirmed)

Status: Fixed (bs17-agent-e)
Regression Test: tests/syntax/threads/threadworker-let-binding-invalid and threadworker-mut-binding-invalid (TYPE_RESOURCE_REQUIRES_RES); tests/rt-behavior/threads/threadworker-res-binding-rt; tests/syntax/threads/threadworker-resource-rules-invalid

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

## Reproduced (base `468ff0f94`, `/tmp/main-dbg-target/release/mfb`)

`LET t2 = t`, `RES t2 = t` and `MUT t3 AS ThreadWorker OF String TO Integer = t`
inside `ISOLATED FUNC spin(t AS ThreadWorker OF String TO Integer, n AS Integer)`
all build and print `thread=42` (`/tmp/bs17e/tw`).

## Resolution

"Is a resource" is decoupled from "has a close op":

- `codegen::resource::is_builtin_resource` — a registry resource or EITHER thread
  view. `builtin_resource_close_function` still answers `None` for a worker view
  (it has no close).
- `ir::verify`'s `TypeEnv::is_resource_base` (a declared/imported resource or a
  built-in one) replaces `close_op_for(..).is_some()` in every "is it a resource"
  question: `is_resource_or_resource_union` (the `RES` binding rule, opaque and
  non-owning parameters, slots, `STATE` access), `is_resource_type`, the move and
  alias dataflow (`RES b = a`, alias-through-call, consuming-parameter reach,
  `RETURN` consumes, transfer), `is_copyable`, `is_defaultable`,
  `provably_data_type`, sendability and `is_resource_name` (mixed-union rule).
  The two sites that ask "does the trap routing CLOSE this local" (the bug-676
  closed set and its `TYPE_TRAP_LOCAL_UNINITIALIZED` exclusion) keep the close-op
  test, because a resource with no close is not closed there.
- `ir::shape::is_resource_type`, `ir::resource_hoist::resource_binding_type` and
  `hir::resource_params` ask `is_builtin_resource`, so the worker entry's first
  parameter is a resource parameter by its type (bug-976's derivation).
- The data-plane remedy (`TYPE_THREAD_RESOURCE_PLANE_REQUIRED`) is offered only to
  a resource that may cross on the resource plane
  (`belongs_on_the_resource_plane`); a worker view on a data plane keeps
  `TYPE_THREAD_NOT_SENDABLE`, and on the resource plane
  `TYPE_THREAD_WORKER_NOT_TRANSFERABLE`, as before. A collection still may not
  hold one (`TYPE_COLLECTION_OWNERSHIP_VIOLATION`).
- Codegen is unchanged: the worker view keeps its own thread arms, and its scope
  cleanup emits no close.

Measured sibling matrix (base → fixed, `/tmp/bs17e/m975/run.sh`):

| program inside the worker | base | fixed |
| --- | --- | --- |
| `LET t2 = t` / `LET t2 AS ThreadWorker … = t` / `MUT t3 … = t` | builds | `TYPE_RESOURCE_REQUIRES_RES` |
| `RES t2 = t`, then `thread::isCancelled(t)` | builds | `TYPE_USE_AFTER_MOVE` (the direct-name move rule every resource follows) |
| `RES a = t; RES b = a; b.state…` (STATE entry param) | thread=47 | thread=47, ncode identical |
| `RES a = t` inside a loop | thread=42 | thread=42, ncode identical; worker arena 0 allocs / 0 frees |
| record field `view AS ThreadWorker …` | builds | `TYPE_RESOURCE_REQUIRES_RES` (as for a `Thread` field) |
| record field `view AS RES ThreadWorker …` | builds | builds, ncode identical |
| `List OF ThreadWorker …` / `Map OF … TO ThreadWorker …` | `TYPE_COLLECTION_OWNERSHIP_VIOLATION` | the same, plus `TYPE_RESOURCE_REQUIRES_RES` for the bare slot |
| `List OF RES ThreadWorker …` | `TYPE_COLLECTION_OWNERSHIP_VIOLATION` | unchanged |
| `t2 = t2` | `TYPE_REQUIRES_COMPARABLE` | unchanged |
| LAMBDA capturing the view | `TYPE_LAMBDA_CAPTURE_UNSUPPORTED` | unchanged |
| `FUNC pass(t AS ThreadWorker …) AS RES ThreadWorker …` | builds | builds, ncode identical |
| bare view, `t.state.hits = 1` | internal error "native code WITH target 'Unknown' is not a record while lowering state assign t" | `TYPE_STATE_INVALID` |
| bare view, `LET h = t.state.hits` | `TYPE_UNKNOWN_VALUE` | `TYPE_UNKNOWN_VALUE` + `TYPE_STATE_INVALID` |
| bare entry param, `RES t2 AS ThreadWorker … STATE W = t` | builds (the binding attached a STATE) | `TYPE_STATE_OPAQUE_NARROWING` — a bare resource parameter is opaque (§15.5); the spec's §16 STATE sentence now says the worker's STATE is declared on the entry parameter |
| worker view on a thread's message plane | `TYPE_THREAD_NOT_SENDABLE` | unchanged |

Regression tests: `threadworker-let-binding-invalid` (LET, inferred and typed),
`threadworker-mut-binding-invalid`, `threadworker-resource-rules-invalid` (move
after alias, bare record field, opaque narrowing, bare STATE assign — the last
crashed the native lowering on the base), `threadworker-res-binding-rt` (alias
shares the STATE across three inner-scope passes and a helper parameter, a `RES`
record field reaches the running worker; prints `thread=45`; `--debug`: the
worker arena frees the record and keeps only the 16-byte STATE, as the base does
for the same shape). Unit tests: `ir::verify::tests::a_let_of_a_worker_view_requires_res`,
`a_worker_view_alias_moves_the_source`, `a_worker_view_is_a_resource_with_no_close_op`,
and `hir::resource_params` (the worker parameter row is now `true`; bug-976 had
pinned it `false`, which pinned this open bug).

Spec/man: `language/04` §4.8, `14` (§14.1 copyable paragraph, §14.2 binding rule),
`15` ("Thread handles are resources too"), `16` (worker view paragraph and the
STATE attachment sentence), `threading/07` (worker view), `mfb man thread`
overview, `mfb man types`.

## Golden impact (not regenerated)

On top of bug-976's 72 (same classes, unchanged), full `scripts/test-accept.sh`:
2 more mismatches, both `build.log`, both one added diagnostic block and no
removed line:

- syntax/threads/func_thread_send_invalid and func_thread_start_invalid: their
  `BadMessage`/`BadStartMessage` record has a bare `handle AS ThreadWorker …`
  field, now `TYPE_RESOURCE_REQUIRES_RES` (the fixtures' own NOT_SENDABLE errors
  are unchanged).

`scripts/artifact-gate.sh … all`: the same 62 diffs as bug-976, no new one.

`scripts/artifact-baseline.sh verify` against the base manifest
(`--targets macos-aarch64`): beyond bug-976's set, more `.nir` files gain a `local`
`resourceOwners` entry for a worker entry parameter (now a resource parameter;
`Local` is what an absent entry meant), and two fixtures that build a source
package with worker functions (rt-behavior/packages/source-package-isolated-rt,
rt-behavior/resources/p121d-state-reach-rt) change `.mir`/`.ncode`/`.nobj`/`.nplan`
only by the package's 16-hex content-hash prefix on its symbol names (the
package's IR now records those entries): `.ncode` is byte-identical after
normalizing that prefix. Committed `.mfp` copies of packages with worker
functions would change the same way on the next `sync-package-mfp.sh`.

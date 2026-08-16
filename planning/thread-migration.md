# `thread` migration plan + the resolver-heavy-package resolution (evidence-backed, 2026-08-16)

## Headline findings
1. **`tls` and `crypto` need NO new infra.** Their custom `BuiltinResolver`s are pure arity/exact-nominal
   dispatch — the clean-room `select()`/overload machinery ALREADY handles multi-overload return variance
   (datetime/process proved it). They were "custom" only because the LEGACY registry couldn't model it.
   Migrate them as multi-overload members (kind/arity-split), like datetime. Do these FIRST (cheap wins).
   - `tls::resolve_call` (tls.rs:309): `exact()` over fixed nominals (TlsSocket/TlsListener/String/Integer/
     List OF Byte/List OF RES tls.TlsSocket); `poll` list-vs-scalar return = arg-type-routed distinct overloads.
   - `crypto::resolve_return_type` (crypto.rs:275): `dispatch_resolve`, fixed-type/arity dispatch.
2. **Only `thread` needs a parametric-type variant.** Its `ThreadResolver` decomposes `Thread OF Msg TO Out` /
   `ThreadWorker OF Msg TO Out` to compute returns. This is a BREADTH change (one variant + mechanical arms),
   NOT a depth change to the bug-443 `select`.

## thread's gap (why the registry can't model it today)
`ParameterType::parse` (types.rs:81) turns `"Thread OF String TO Integer"` into an OPAQUE `Named(<blob>)`
(only List/Set/Map/FUNC decompose). So: no structured variant, parse doesn't decompose, `unify` has no arm
(hits the `(leaf,_)` fallback → accepts blindly, binds nothing), `substitute` can't rebuild a handle.

## The minimal extension — `ParameterType::ThreadHandle`
```rust
// src/types.rs
ThreadHandle { worker: bool, msg: Box<ParameterType>, res: Box<ParameterType>, out: Box<ParameterType> }
```
- `res` is an always-present slot defaulting to `Nothing` (mirrors legacy `thread_parts_full` defaulting
  message to "Nothing"); `name()` elides the RES clause when `res == Nothing`. No "optional slot" concept needed.
- Six mechanical arms, each parallel to the existing List/Map/Func arms:
  | fn (file:line) | change |
  |---|---|
  | `parse` (types.rs:81) | recognize `Thread OF `/`ThreadWorker OF ` prefixes; reuse `thread_parts_full`/`split_thread_types`/`type_prefix_len` splitters (thread.rs:452/474/541), recurse parse; res→Nothing when absent |
  | `name` (types.rs:~124) | render via `format_thread_type` semantics (RES elided when Nothing) |
  | `is_scalar` | ThreadHandle is NOT scalar (so leaf_matches never mis-accepts) |
  | `unify` (registry/mod.rs:1185) | arm `(ThreadHandle p, ThreadHandle c)`: require `p.worker==c.worker`, unify msg/out, unify res via `resource_base_eq` (STATE-agnostic); re-occurring `Var(Msg)` (send) + `Unknown` wildcard already work |
  | `substitute` (registry/mod.rs:1256) | rebuild ThreadHandle with substituted slots (lets `start` return a fresh parent handle; waitFor/receive/accept return Var(out/msg/res)) |
  | `contains_var` (registry/mod.rs:1280) | recurse the three slots |
- `select()` (registry/mod.rs:328) + bug-443 strict/lenient dispatch UNTOUCHED — the variant rides existing recursion like Func/MapOf.
- Also extend `is_builtin_type`/`resolve_type`/`qualified_builtin_type` (registry/mod.rs:1061/959/1073) to recognize
  the head token before ` OF ` of a parametric spelling (else `Thread OF X TO Y` isn't accepted as a builtin type).

## thread members (12 user-callable; internal transferResource/acceptResource/emitResource/readResource are lowered-only)
- Extract-into-return: `start`→build parent handle (nested-Func extraction + kind flip + optional RES; hardest),
  `waitFor`→Out, `receive`→Msg, `accept`→Res. Cross-param constraint: `send`→Msg==arg1 (or Unknown), `transfer`→Res base-match.
  Const-return guards: `isRunning`/`cancel`/`poll`(parent), `isCancelled`(worker), `sleep`(either), `openStdIn`/`closeStdIn`(opt parent).
- Parent/worker "either" members → register TWO kind-split overloads (datetime pattern), zero extra infra.
- Resource plane (`transfer`/`accept` → parent-vs-worker `*Resource`): stage 1 (transfer→transferResource) = `Body::Rewrite`
  descriptor data (deletes `thread_resource_plane_target` ir/lower.rs:2230); stage 2 (kind direction split,
  builder_values.rs:1840-1921) folds into the kind-split overloads' rewrite targets ONCE the ThreadHandle variant exists.

## thread types + resources (MODELING DECISION — flag)
Thread/ThreadWorker are `TypeKind::Opaque`, NOT RES-table resources (no ResourceInfo row; cleanup via
`builder_thread_cleanup.rs` + `thread.drop` op, not the RES sendable/close_may_fail machinery). Recommendation:
model as opaque type NAMES (+ the is_builtin_type parametric-prefix extension), NOT `add_resource`; keep `thread.drop`
cleanup wiring shared. (Do NOT blindly copy the process `add_resource` shape — Thread doesn't behave like File/Socket/Process.)

## io/stdin coupling — CLEAN
`stdin_broadcast.rs` + `_mfb_rt_stdin_next_byte` + the main-thread auto-subscription STAY shared (io, migrated,
references the symbol by name). Only the `openStdIn`/`closeStdIn` MEMBERS are thread surface (const Nothing, opt parent guard).

## Execution order
1. `tls`, then `crypto` — no infra; multi-overload members (validate overload-return-variance, mirror datetime/process).
2. `ParameterType::ThreadHandle` variant + 6 arms + is_builtin_type parametric recognition — self-contained infra,
   port the thread.rs:719-1006 round-trip test battery against parse/name/unify/substitute DIRECTLY before wiring members.
3. `thread` registration — kind-split overloads + `Body::Rewrite` resource plane; delete `thread_resource_plane_target`
   + the builder_values kind-branches; keep stdin/thread.drop/runtime specs shared.
4. Contingency only: a `RegistryPackage` structural-resolver hook (defeats the no-custom-resolver goal) if step 2's
   round-trip / `start` extraction proves too costly.

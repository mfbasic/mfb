# plan-72-Y: thread package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/thread.rs` (840 LOC, 7 metadata helpers, 0 source
glue, 1 builtin-type helper, 0 custom-resolver helpers, 0 fixtures) to a
`pub(crate) static THREAD: BuiltinModule`.

References: plan-72 overview, `src/builtins/thread.rs`,
`memory/thread-resource-plane-split.md`,
`memory/glibc-musl-thread-entry-alignment.md`.

## Goal

- `thread::THREAD` descriptor exists and mirrors every current metadata
  helper plus the builtin-type entry.
- Legacy free functions in `thread.rs` become wrappers over `THREAD`.
- Parity tests cover every function and the builtin type.

## Non-goals

- Do not change thread-resource plane split semantics
  (`[[thread-resource-plane-split]]`).
- Do not change trampoline alignment
  (`[[glibc-musl-thread-entry-alignment]]`).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/thread.rs` has 7 descriptor-owned helpers and one builtin
type. There are no fixtures under
`tests/{syntax,rt-behavior,byte-identity}/*/thread/`; parity is proven by
`cargo test` (unit + integration tests exercising the thread runtime).

## Phases

### Phase Y1 — descriptor and wrappers

- [x] Add `pub(crate) static THREAD: BuiltinModule` with every function,
      overload, parameter (canonical + aliases), argument types, return
      type, implementation, and default.
- [x] Add `BuiltinType` entry for the thread builtin type. (Two: `Thread`
      and `ThreadWorker`, both `Opaque`.)
- [x] Rewrite the 7 metadata helpers as wrappers over `THREAD`. (`is_thread_call`
      → `DefaultResolver::contains`, `arity` → `DefaultResolver::arity`;
      `call_param_names`/`expected_arguments`/`resolve_call`/`is_builtin_type`
      stay hand-authored and are pinned/kept per the notes below — see Corrections.)
- [x] Register `THREAD` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests for every `thread.*` name and the builtin type.

Acceptance: `cargo test` passes. Because there are no thread fixtures
under `tests/`, acceptance for this letter is `cargo test` only unless a
new thread fixture is added. (`cargo test --bin mfb builtins::thread` → 21
passed incl. `parity_matches_descriptor`.)
Commit: 17cc0d9eb

## Validation

- `cargo test`

## Corrections

- **`resolver: None`, not a custom resolver (like `net`).** The overview lists
  thread with `0 custom` helpers; its returns ARE argument-dependent (a handle's
  message/output/resource is parsed structurally from its type string), but that
  is `resolve_call`'s job, not a `default_argument_padding`/`implementation_name`
  hook. Following the `net` precedent (fixed metadata via descriptor, `resolve_call`
  hand-authored), thread carries every overload as `ReturnType::Custom` and keeps
  `resolve_call` unchanged. `DefaultResolver::return_type_name` then returns `None`
  for every call, matching thread's *absence* of a `call_return_type_name` helper,
  so parity holds with no resolver and no resolver samples. BB will route the
  computed-return packages (`net`, `thread`) through the registry uniformly.
- **Only 2 of the "7 metadata helpers" became thin `DefaultResolver` wrappers**
  (`is_thread_call`, `arity`). `call_param_names` stays a `&'static` literal
  (pinned equal to `THREAD` by `parity_matches_descriptor`); `resolve_call` and
  `expected_arguments` are argument-dependent / `"or"`-phrased and stay
  hand-authored (checked by the existing `resolve_*` and `expected_arguments_all_arms`
  tests); `is_builtin_type` stays hand-authored because it also accepts the
  parametric `Thread OF ...` / `ThreadWorker OF ...` forms the descriptor type
  list cannot enumerate. This mirrors every prior letter (`net`, `money`).

# plan-31-A: `os` package — environment variables

Last updated: 2026-07-08
Overall Effort: large (3h–1d) — the whole plan-31 `os` feature
Effort: medium (1h–2h)

This sub-plan introduces a new built-in `os` package and lands its first,
self-contained slice: **process environment variables**. It gives MFBASIC
programs a way to read, test, set, unset, and enumerate the host environment —
capabilities nothing in the stdlib exposes today. A correct implementation lets
a program call `os::getEnv("HOME")` and observe the same string the host shell
would, raise `ErrNotFound` for a truly-unset name (or take a caller default via
`os::getEnvOr`), and round-trip a value it wrote with `os::setEnv`.

`os` is a built-in package (`IMPORT os`, no manifest dependency), modelled on
`fs`: compile-time metadata in `src/builtins/os.rs`, runtime-helper specs in
`src/target/shared/runtime/`, and per-backend libc emission in each target's
`code.rs`/`plan.rs`.

It complements:

- `./mfb spec diagnostics error-codes` (reuses the shared `7-705-xxxx`
  standard-package error block — no new codes; canonical specs under
  `src/docs/spec/**`)
- `./mfb spec package` (package registration / built-in package list)
- `./mfb man fs` (the precedent this package mirrors for structure and wiring)

## 1. Goal

- A new built-in `os` package resolvable via `IMPORT os`, exposing:
  - `os::getEnv(name AS String) AS String` — the variable's value, or **raise
    `ErrNotFound`** when unset.
  - `os::getEnvOr(name AS String, fallback AS String) AS String` — returns
    `fallback` when the variable is unset (never raises for absence). Mirrors the
    established `collections::getOr(map, key, fallback)` naming.
  - `os::hasEnv(name AS String) AS Boolean` — `TRUE` iff the variable is set.
  - `os::setEnv(name AS String, value AS String)` — SUB; sets/overwrites.
  - `os::unsetEnv(name AS String)` — SUB; removes the variable (no-op if absent).
  - `os::environ() AS Map OF String` — a snapshot of every variable as a
    `name -> value` map.
- `getEnv`/`getEnvOr`/`hasEnv`/`environ` observe values set by the host **and**
  by prior `setEnv`/`unsetEnv` in the same process.

### Non-goals (explicit constraints)

- **No new language surface.** `os::` is ordinary qualified-package call syntax;
  no new keywords, operators, or statement forms.
- **No new value/copy/move semantics.** Returned `String`, `Boolean`, and
  `Map OF String` are ordinary owned values under the existing rules; the
  package holds no resource handles.
- **No layout/ABI change.** No struct/record layout changes; golden output for
  existing programs is unaffected.
- **No new error-code block.** Reuse the shared `7-705-xxxx` codes
  (`ErrNotFound`, `ErrInvalidArgument`, `ErrOutOfMemory`); do not mint an `os`
  prefix.
- **Env mutation is not synchronized.** `setEnv`/`unsetEnv` mutate
  process-global state and are not race-free against concurrent `getEnv` in
  another `thread::` worker (the classic `getenv`/`setenv` data race). This is a
  documented caveat, not a feature to solve here.

## 2. Current State

- Nothing in the stdlib reads or writes the process environment. `fs` reaches
  the cwd/temp dir (`fs::currentDirectory`, `fs::tempDirectory`) but never
  `getenv`.
- Built-in package precedent is fully worked out in `fs`:
  - **Compile-time metadata** — `src/builtins/fs.rs` provides `is_fs_call`,
    `call_param_names`, `call_return_type_name`, `resolve_call`,
    `expected_arguments`, `arity`. A parallel `src/builtins/os.rs` is the model.
  - **Runtime-helper specs** — `src/target/shared/runtime/fs_specs.rs` declares
    helper symbols like `_mfb_rt_fs_fs_currentDirectory`. `os` needs an
    `os_specs.rs` sibling.
  - **Per-backend libc emission** — `fs.currentDirectory` emits a `getcwd`
    call in `src/target/macos_aarch64/code.rs`, `src/target/linux_aarch64/code.rs`,
    `src/target/linux_x86_64/code.rs`, with the import registered in each
    `plan.rs` (`libc_import("getcwd", …)` / `("libSystem", "_getcwd")`). The
    in-flight `src/target/linux_riscv64/` backend must gain the same emission.
- The shared standard-package error block already defines `ErrNotFound`
  (`77050004`), `ErrInvalidArgument` (`77050002`), and `ErrOutOfMemory`
  (`77010001`) — see `src/docs/spec/diagnostics/02_error-codes.md`.
- `.ai/compiler.md` governs the runtime-completion gate, validation, and
  function tests for anything touching built-ins/codegen/runtime helpers.

## 3. Design Overview

Three layers, mirroring `fs`:

1. **Frontend metadata** (`src/builtins/os.rs`, registered alongside the other
   built-in packages): recognises the six `os::` names above and publishes their
   arity/param-names/return-types. Each name is a distinct function (no
   arg-count overloads), so `resolve_call` is a straight name→signature lookup.
2. **Runtime helper specs** (`src/target/shared/runtime/os_specs.rs`): declares
   the helper symbols the codegen calls. The env helpers need libc `getenv`,
   `setenv`, `unsetenv`, plus the global `environ` pointer to walk for
   `os::environ()`.
3. **Per-backend emission**: each target lowers the `os.*` names to the libc
   calls, marshalling MFBASIC `String` args into NUL-terminated buffers (reuse
   the fs path-marshalling helpers in `src/target/shared/code/`) and marshalling
   results back into `String`/`Boolean`/`Map OF String`.

Correctness risk concentrates in `os::environ()` (walking the `char **environ`
NUL-terminated array and splitting each `KEY=VALUE` entry into map entries) and
in the missing-variable distinction (a NULL `getenv` return must become
`ErrNotFound` for `getEnv` but the `fallback` value for `getEnvOr`).

## 4. Detailed Design

### 4.1 Frontend metadata (`src/builtins/os.rs`)

Copy the shape of `src/builtins/fs.rs`:

- `is_os_call(name)` recognises `os.getEnv`, `os.getEnvOr`, `os.hasEnv`,
  `os.setEnv`, `os.unsetEnv`, `os.environ`.
- `arity`: `getEnv`/`hasEnv`/`unsetEnv` → 1 arg; `getEnvOr`/`setEnv` → 2 args;
  `environ` → `(0, 0)`. All fixed-arity.
- `call_param_names` / `expected_arguments`: `("name")`, `("name", "fallback")`,
  `("name", "value")`, `()`.
- `call_return_type_name`: `getEnv`/`getEnvOr`→`String`, `hasEnv`→`Boolean`,
  `environ`→`Map OF String`; `setEnv`/`unsetEnv` are SUBs (no return).
- `resolve_call` is a straight name→signature lookup (no overloads to
  disambiguate).
- Register `os` in the built-in package list so `IMPORT os` resolves and the
  package shows in `mfb man`.

### 4.2 Runtime helpers (`src/target/shared/runtime/os_specs.rs`)

Declare helper specs analogous to `fs_specs.rs`. Two viable shapes — pick per
Open Decision D1:

- **Thin**: one helper per op (`_mfb_rt_os_getEnv`, `_mfb_rt_os_setEnv`,
  `_mfb_rt_os_environ`, …), each a small hand-emitted sequence calling the libc
  primitive, matching how `fs.currentDirectory` is a single emitted `getcwd`
  call.
- `hasEnv` = `getenv(name) != NULL`; no separate primitive.
- `getEnvOr` = `getenv`; NULL → materialise the `fallback` String.
- `getEnv` = `getenv`; NULL → `ErrNotFound`.

### 4.3 Per-backend emission

For each of `macos_aarch64`, `linux_aarch64`, `linux_x86_64`, `linux_riscv64`:

- Register the libc imports in `plan.rs` (`getenv`/`setenv`/`unsetenv`; macOS
  underscored `_getenv` etc.; `environ` as a data symbol — on macOS via
  `_NSGetEnviron()` since the `environ` symbol is not directly linkable in a
  PIE, so register `_NSGetEnviron`).
- Emit the call sequences in `code.rs`, reusing the existing String→NUL-buffer
  marshalling from `src/target/shared/code/` (the same helpers fs paths use) and
  the result String allocation path.
- `os::environ()`: walk the `char **` array to its NULL terminator; for each
  entry find the first `=`, split into key/value, insert into a freshly built
  `Map OF String`.

## Layout / ABI Impact

None. No record/struct layout changes, no new resource type, no change to
existing helper symbols. `Map OF String` / `List OF String` returns use existing
collection construction paths. Golden output for existing programs is unchanged;
the only new symbols are the `os` helpers and their libc imports.

## Phases

### Phase 1 — Frontend metadata + resolution (no codegen)

Land the package surface so `IMPORT os` and the six calls typecheck and resolve,
ahead of any emission. Safe to land alone: with no backend, programs that call
`os::` fail to *compile to native* but the frontend/typecheck/overload behaviour
is fully testable.

- [ ] Create `src/builtins/os.rs` with `is_os_call`, `arity`,
  `call_param_names`, `call_return_type_name`, `resolve_call`,
  `expected_arguments` for the six names; unit tests mirroring
  `src/builtins/fs.rs` (`every_name_has_consistent_metadata`, signature lookup).
- [ ] Register `os` in the built-in package list / dispatch (wherever `fs` is
  registered) so `IMPORT os` resolves and it appears in `mfb man`.
- [ ] Tests: `tests/func_os_{getEnv,getEnvOr,hasEnv,setEnv,unsetEnv,environ}_valid/**`
  + `_invalid/**` — covering arity errors, wrong arg types, and `IMPORT os`
  resolution.

Acceptance: signature-lookup and arity/type-error unit tests pass; a program
`IMPORT os` + `os::getEnv("X")` type-checks and produces the correct
compile-time diagnostics for misuse. (Native emission proven in Phase 3.)
Commit: —

### Phase 2 — Runtime helper specs

Declare the helper symbols and their libc dependencies without yet wiring every
backend, so the spec layer is reviewable in isolation.

- [ ] Add `src/target/shared/runtime/os_specs.rs` declaring the env helper
  symbols and required libc imports (`getenv`, `setenv`, `unsetenv`, environ
  access), following `fs_specs.rs`.
- [ ] Wire `os_specs` into the shared runtime module (`src/target/shared/runtime/`
  mod registration) alongside `fs_specs`.

Acceptance: `cargo build` clean with the specs referenced; a unit test asserts
every `os` frontend name has a matching runtime-helper spec (parity check, as fs
does between metadata and specs).
Commit: —

### Phase 3 — Per-backend emission (highest-risk codegen last)

Emit the libc call sequences on every backend and prove real behaviour.

- [ ] Emit `getEnv`/`hasEnv`/`setEnv`/`unsetEnv`/`environ` in
  `src/target/macos_aarch64/code.rs` + imports in `plan.rs` (`_getenv`,
  `_setenv`, `_unsetenv`, `_NSGetEnviron`).
- [ ] Same for `src/target/linux_aarch64/{code,plan}.rs`.
- [ ] Same for `src/target/linux_x86_64/{code,plan}.rs`.
- [ ] Same for `src/target/linux_riscv64/{code,plan}.rs` (coordinate with the
  in-flight plan-99 backend; see [[plan-99-rv64-impl]]).
- [ ] `os::environ()` map-build + `KEY=VALUE` split path emitted and tested.

Acceptance (runtime proof, not golden-only): a compiled program prints
`os::getEnv("MFB_OS_TEST")` matching a value exported in its environment;
`os::getEnv` raises `ErrNotFound` for an unset name while `os::getEnvOr` returns
the fallback; `os::setEnv`+`os::getEnv` round-trips; `os::environ()` contains a
just-set key. Validate on each backend that is locally buildable and on the
riscv64 remote (`ssh -p 2229`, [[plan-99-rv64-impl]]). Acceptance suite
(`scripts/test-accept.sh`) passes.
Commit: —

## Validation Plan

- Function tests:
  `tests/func_os_{getEnv,getEnvOr,hasEnv,setEnv,unsetEnv,environ}_valid/**`
  and `_invalid/**`.
- Runtime proof: the Phase 3 program above (env round-trip + `ErrNotFound` vs
  fallback + `environ` snapshot), run natively per backend.
- Doc sync: new `src/docs/man/` pages for the `os` package overview + each
  function (`scripts/update_man.sh` / `scripts/update_man_package.sh`, templates
  per `.ai/man_*_template.md`); note the reused `7-705-xxxx` codes in the man
  Errors sections. No `mfb spec diagnostics` change (no new codes) beyond adding
  `os` to any package inventory the spec enumerates.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **D1 — helper granularity**: one thin helper per op (recommended — matches
  `fs.currentDirectory`'s single-call emission, easy to reason about per
  backend) vs. a single dispatched `_mfb_rt_os_env` helper taking an op code
  (fewer symbols, more indirection). Recommend thin.
- **D2 — `getEnv` on unset**: raise `ErrNotFound` (recommended — consistent
  with `fs` readers raising `ErrPathNotFound`, and `os::getEnvOr` +
  `os::hasEnv` give non-raising paths) vs. return empty string (loses the
  set-to-empty vs. unset distinction). Recommend raise.
- **D3 — macOS `environ` access**: `_NSGetEnviron()` (recommended — the
  supported PIE-safe accessor) vs. linking `environ` directly (fails in a
  position-independent executable). Recommend `_NSGetEnviron`.

## Non-Goals

- Well-known directories (`homeDirectory`, `documentsDirectory`, …) — dropped
  from v1 by request.
- `os::exit` — redundant with the existing `EXIT PROGRAM n` statement
  (`mfb spec language`, §08 error-model: runs stack-wide lexical cleanup then
  exits with code `n`).
- Process/platform introspection (`args`, `pid`, `name`, `arch`, `hostName`,
  `userName`, `executablePath`, `cpuCount`) — see [[plan-31-B-os-process]].
- Subprocess spawn/exec — deferred; a future `process::` package, not `os`.

## Summary

The engineering risk is the per-backend libc emission (four backends, one
in-flight) and the `os::environ()` array walk + `KEY=VALUE` split; everything
else is thin `getenv`/`setenv` wrapping over the well-worn `fs` machinery.
Nothing about layout, ABI, copy/move semantics, or existing golden output
changes. Process/platform introspection is carved off into sub-plan B so each
half stays a one-sitting, medium-sized landing.

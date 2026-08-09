# plan-90-A: `process` package — core spawn & lifecycle (Unix)

Last updated: 2026-08-08
Overall Effort: x-large (1d–3d) — the whole `plan-90` `process` feature
Effort: large (3h–1d)
Depends on: nothing (this is the anchor sub-plan)

This sub-plan creates the built-in `process` package and its `Process` resource,
and delivers a working end-to-end vertical slice on the **Unix backends**
(macOS-aarch64, Linux x86_64 / aarch64 / riscv64): spawn a child, learn its pid,
ask whether it is still running, wait for its exit code, close its stdin, and —
critically — reap it automatically when the resource drops. A correct
implementation lets a program run `process::spawn(["echo", "hi"])`, call
`waitFor` and get `0`, and leave behind **no zombie** when the `Process` goes out
of scope. This is the riskiest, load-bearing slice: it proves the resource
plumbing, the fork/exec/pipe/waitpid machinery, and the drop-reap policy all
work before any I/O, signal, or Windows work is layered on.

The subsequent sub-plans are: **B** streaming I/O (`send`/`receive`/`poll` +
`Stream`), **C** signals & `detach` (`signal`/`didSignal` + `Signal`), **D** the
Windows backend, **E** cross-target finalization.

References:

- `./mfb man net` and `src/builtins/net.rs` — the closest resource-package
  precedent (a native resource returned from a builtin that TRAPs on failure).
- `src/target/shared/code/audio/mod.rs:1` — the per-OS native-backend dispatch
  pattern this package copies (`mod macos; mod alsa; mod windows;`).
- `src/docs/spec/memory/03_heap-values.md:160` — the canonical plan-80 96-byte
  resource-record envelope (tag@0/handle@8/closed@16/state@24).
- `planning/completed/plan-31-B-os-process.md:43,213` — the `os` plan that
  explicitly deferred subprocess spawn/exec to "future `process::`" (this).
- `./mfb spec diagnostics error-codes` — the `77-BB-NNNN` error-code scheme.

## Prerequisites

These are a precondition on the whole `plan-90` feature (stated once here; B–E
point back to this table).

| Must be true | Command | Status |
|---|---|---|
| No prior `process` package exists (net-new, no partial landing to reconcile) | `rg -l 'builtins/process' src/ ; ls src/builtins/process.rs 2>/dev/null` → no matches | MET |
| Resource tag `10` is free | `grep -n 'RESOURCE_TAG_' src/target/shared/code/error_constants.rs` → highest real tag is `9` (AUDIO) | MET |
| A riscv64 remote is reachable for runtime proof | `ssh -p 2229 <rv64-host> true` (per plan-31-B) | MET — box 2229 reported online (re-verify at Phase 4) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before continuing and before deciding to stop.

Everything below is written against the world where these hold.

## 1. Goal

- A new built-in `process` package registered exactly like `net`/`audio`, whose
  `Process` is a native resource using resource **tag 10**.
- Working on all four Unix backends:
  - `process::spawn(args AS List OF String) AS Process`
  - `process::spawn(args AS List OF String, cwd AS String, env AS Map OF String TO String, envReplace AS Boolean) AS Process`
  - `process::shell(cmd AS String) AS Process` — runs the platform default shell
    (`sh -c` on Linux, `bash -c` on macOS).
  - `process::pid(p AS Process) AS Integer`
  - `process::isRunning(p AS Process) AS Boolean`
  - `process::waitFor(p AS Process) AS Integer` — blocks, returns the exit code,
    `-1` if the child died on a signal. Caches the result.
  - `process::close(p AS Process) AS Nothing` — closes the child's stdin only.
- **Drop policy: a live `Process` that goes out of scope is force-killed
  (`SIGKILL`) and reaped** (`waitpid`), so no zombie is left and drop never
  blocks indefinitely.
- **Spawn failure TRAPs** (raises an Err that propagates as a trap), matching
  `net::connectTcp`/`fs::open`.

### Non-goals (explicit constraints)

- **No I/O here** — `send`/`sendBytes`/`receive`/`receiveBytes`/`poll` and the
  `Stream` enum are sub-plan B. Stdout/stderr/stdin pipes ARE created at spawn
  (Phase 3) so B has fds to read, but no read/write builtin is exposed yet.
- **No signals here** — `signal`/`didSignal` and the `Signal` enum are sub-plan
  C. (`waitFor` returning `-1`-on-signal is in scope; interpreting *which*
  signal is not.)
- **No Windows here** — sub-plan D. This sub-plan's native backend is Unix-only;
  the `process/windows.rs` module is created as a stub-free `unimplemented`-free
  file only in D, not here. Until D lands, a Windows build does not offer
  `process`.
- No new language surface, no value/copy/move semantic change, no change to the
  96-byte resource envelope layout, no golden change for existing programs.

## 2. Current State

- **No process spawning exists.** `std::process::Command`/`fork`/`waitpid`
  appear only in the compiler's own build/test tooling (`src/cli/build/test_mode.rs:57`,
  `src/os/linux/appimage/mod.rs:412`), never in emitted-program runtime. The `os`
  package is read-only and nullary (`src/builtins/os.rs:13`).
- **Resource envelope** is fixed (plan-80): 96-byte arena record, header
  tag@0 / handle@8 / closed@16 / state@24 — constants at
  `src/target/shared/code/error_constants.rs:817,820,879,886,869`. A new resource
  allocates this envelope and stamps tag/handle/closed; precedent
  `src/target/shared/code/net/mod.rs:392`.
- **Resource tags** live at `error_constants.rs:912-921` (`FILE`=1 … `AUDIO`=9,
  `NATIVE`=255). Mirrored in the spec table `03_heap-values.md:199` and in
  `src/binary_repr/{mod.rs:127,sections.rs:108,reader.rs:896}`.
- **Package registration** is three edits: `src/builtins/descriptor.rs:631`
  (`REGISTRY`), `src/builtins/mod.rs:100` (`is_builtin_import`) and `:1084`
  (`ALL_BUILTIN_PACKAGES`); resource registration is `src/builtins/resource.rs:138`
  (`BUILTIN_RESOURCES`, plus the enumerating test at `:360`).
- **Native backend layout**: `src/target/shared/code/audio/` is the model —
  `mod.rs` dispatches a runtime-helper body to `macos.rs`/`alsa.rs`/`windows.rs`
  (`audio/mod.rs:118`). Per-OS syscall divergence goes through the `platform`
  object (`net/mod.rs:740` `platform.emit_syscall`/`emit_errno`).
- **Runtime helper specs**: a `RuntimeHelper` enum variant
  (`src/target/shared/runtime/mod.rs:4`) + a per-package spec file modeled on
  `net_specs.rs:3` (a resource-returning helper is `returns: "Socket"`), wired
  through `catalog.rs:298`.
- **Failure = TRAP**: a resource-returning builtin raises an Err
  (`src/target/shared/code/error_result.rs:97` `lower_make_error_result`) that
  propagates as a trap; net passes `ERR_*_CODE/_SYMBOL` pairs at the failure site
  (`net/mod.rs:942`).
- **Drop/cleanup dispatch**: `src/target/shared/code/builder_resource_cleanup.rs:16`
  resolves the close op from `resource_close_function`; scope-drop copy-insertion
  frees owned resources (memory `scope-drop-frees`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Public `process::` functions in the API spec (whole feature) | 13 | count of the API block: spawn(×2)/shell/send(×2)/sendBytes(×2)/receive(×2)/receiveBytes(×2)/poll(×2)/close/isRunning/didSignal/waitFor/signal/detach/pid → 13 distinct names |
| Functions in THIS sub-plan | 5 names (7 overloads) | spawn(×2), shell, pid, isRunning, waitFor, close |
| Native codegen backends needing import registration | 5 | `ls -d src/target/*/` minus shared/variant dirs → macos_aarch64, linux_x86_64, linux_aarch64, linux_riscv64, win_x86_64 |
| Unix backends in scope for THIS sub-plan | 4 | the above minus win_x86_64 |
| Next free resource tag | 10 | `grep RESOURCE_TAG_ error_constants.rs` → tags 1-9 used, 255 reserved |
| Free error-code blocks | 7704 / 7706 / 7708 | `grep -oE 'ERR_..._CODE: &str = "77([0-9]{2})' error_constants.rs \| sort -u` → 7701/7702/7703/7705/7707 used |
| Existing byte-identity package dirs (none for process) | 24 | `ls tests/byte-identity/` |

### Verified properties

- **Tag 10 is unused** — VERIFIED by the grep above (highest real tag 9).
- **The 96-byte envelope has room for a Process** — VERIFIED against
  `03_heap-values.md:160`: header ends at offset 32, leaving 64 bytes of
  type-specific tail for the three pipe fds + cached exit-state; no envelope
  change needed.
- **Drop-reap is expressible in the existing cleanup path** — UNVERIFIED. The
  close-function dispatch exists, but whether a close op can both `kill`+`waitpid`
  (vs. a plain fd close) must be confirmed by reading
  `builder_resource_cleanup.rs` end-to-end in Phase 3; treat as the sub-plan's
  central design risk.

## 3. Design Overview

Four layered pieces, landing in this order so the cheapest plumbing proves out
before the OS mechanism it feeds:

1. **Package + type plumbing (no OS calls).** Register `process`, declare the
   `Process` resource type + tag 10, wire the descriptor/resolver/resource
   registry, add the source companion `.mfb`. Deliverable on its own: `Process`
   resolves as a type name; no callable yet.
2. **Frontend metadata for the 5 functions.** Arity, return types, overloads in
   `src/builtins/process.rs`.
3. **Native Unix backend — the mechanism.** `process/mod.rs` + `process/unix.rs`:
   fork + exec + three pipes (stdin/stdout/stderr, created now so B can read
   them), allocate & stamp the 96-byte record, cache pid. `waitFor`/`isRunning`
   via `waitpid(WNOHANG?)` with exit/signal decoding cached into the record.
   `close` closes the stdin fd. **Drop = SIGKILL + waitpid** wired through the
   resource cleanup path.
4. **Error codes + TRAP.** A `process` error block (recommend **7708**:
   `ErrSpawnFailed`; reuse `ErrResourceClosed`/`ErrInvalidArgument`/`ErrIo`) added
   to `02_error-codes.md` + `error_constants.rs`, raised at the fork/exec failure
   site.

**Where correctness risk concentrates:** the fork/exec/pipe sequence (fd leaks,
exec-failure signalling back to the parent, close-on-exec on the pipe ends the
parent keeps) and the **drop-reap** wiring (must not block, must not zombie, must
run on every drop path including trap unwinding). Both land in Phase 3, behind
runtime tests.

**Byte-identity is NOT this plan's gate.** This adds new runtime behavior and new
codegen; existing programs' goldens must stay byte-identical (regression guard),
but the new `process` fixtures are validated by **runtime behavior** (spawn a
child, observe exit code / pid / no-zombie), not by byte comparison. Per the
Windows non-goal (memory `windows-byte-identity-is-a-nongoal`), no byte-identity
is chased on Windows at all; that is sub-plan D's execution-only concern.

**Rejected alternatives:**

- *Drop = wait-only (no kill).* Rejected: a child that never exits makes the
  resource drop hang forever. Kill-then-reap is the only bounded policy.
- *Drop = detach (leak the child).* Rejected as the default: silently leaks
  processes. Relinquishing without killing is opt-in via `detach` (sub-plan C).
- *posix_spawn instead of fork/exec.* Rejected: `posix_spawn`'s file-action /
  cwd handling is less uniform across macOS/Linux than an explicit
  fork+`chdir`+`dup2`+`execvp`, and the explicit form is what B/C/D extend.

## 4. Detailed Design

### 4.1 Package & type plumbing

- New `src/builtins/process.rs`: `PROCESS_TYPE: &str = "Process"`, the
  `BuiltinModule` static `PROCESS`, `include_str!("process_package.mfb")`.
- Register in `descriptor.rs:631` REGISTRY, `mod.rs:100`/`:1084`,
  `resolver` BUILTIN_TYPES, and `is_resource_type` (`mod.rs:189`).
- `resource.rs:138` `BUILTIN_RESOURCES`: add `ResourceInfo { close_function:
  "process.__drop", sendable: <see C>, close_may_fail: false, kind: Builtin }`;
  extend the enumerating test at `:360`.
- New resource tag: `error_constants.rs` `RESOURCE_TAG_PROCESS = "10"`; mirror in
  `03_heap-values.md:199` and `src/binary_repr/{mod.rs,sections.rs,reader.rs}`.
- New `src/builtins/process_package.mfb`: header comment (source-companion idiom,
  copy `net_package.mfb:1-8`) + the `EXPORT` type declaration for `Process`.
  (Enums `Stream`/`Signal` are added by B/C when first used, to avoid dead
  surface.)

### 4.2 Frontend metadata (`src/builtins/process.rs`)

`is_process_call`, `arity` (spawn 1 and 4; shell 1; pid/isRunning/waitFor/close
1), `call_return_type_name` (spawn/shell→`Process`, pid/waitFor→`Integer`,
isRunning→`Boolean`, close→`Nothing`), `expected_arguments`, and `resolve_call`
overload selection for the two `spawn` forms.

### 4.3 Native Unix backend

- New `src/target/shared/code/process/mod.rs` (dispatch, copy `audio/mod.rs:118`
  shape) + `process/unix.rs` (the fork/exec/pipe/waitpid emission). `windows.rs`
  is created empty-of-behavior only in sub-plan D.
- `RuntimeHelper::Process` variant (`runtime/mod.rs:4`) + name map (`:32`) +
  `helper_for_call` (`:128`); new `runtime/process_specs.rs` (helpers for spawn,
  shell, pid, isRunning, waitFor, close, and the internal `__drop`), registered
  in `catalog.rs:298`.
- **spawn**: create 3 pipes; `fork`; child `dup2`s pipe ends to 0/1/2, applies
  `cwd` (`chdir`) and `env` (replace vs. merge per `envReplace`), `execvp`;
  parent closes the child ends, allocates the 96-byte record, stamps tag=10,
  handle=pid, closed=0, and stores {stdin-w, stdout-r, stderr-r fds, cached
  exit-state = "not yet reaped"} in the type tail. exec failure in the child is
  reported to the parent over a close-on-exec self-pipe → parent raises
  `ErrSpawnFailed` (TRAP).
- **waitFor**: `waitpid(pid, &status, 0)`; decode `WIFEXITED`→`WEXITSTATUS`,
  `WIFSIGNALED`→`-1`; cache exit code + raw status into the record; idempotent
  (a second `waitFor` returns the cached value, does not re-`waitpid`).
- **isRunning**: `waitpid(pid, &status, WNOHANG)`; `0` → running; reaped →
  cache exit-state, return false. Must cache so a later `waitFor` sees it.
- **pid**: read cached handle.
- **close**: close the stdin-write fd only; mark it closed in the tail (double
  close is a no-op). Does not touch the resource `closed` flag (the child is
  still alive).
- **`__drop` (cleanup op)**: if not yet reaped → `kill(pid, SIGKILL)` then
  `waitpid(pid, _, 0)`; close any still-open pipe fds; set the record `closed`
  bit. Wired via `builder_resource_cleanup.rs:16`.

### 4.4 Error codes

Add block **7708** to `src/docs/spec/diagnostics/02_error-codes.md` and the
paired `ERR_SPAWN_FAILED_CODE/_SYMBOL` to `error_constants.rs`; reuse
`ERR_RESOURCE_CLOSED_*` for operating on a dropped `Process` and
`ERR_INVALID_ARGUMENT_*` for an empty `args` list.

## Compatibility / Format Impact

- **New**: resource tag 10, error block 7708, the `process` package + type. No
  change to the 96-byte envelope, to any existing tag, or to existing goldens.
- The `binary_repr` sentinel additions must round-trip; existing `.mfp`/bytecode
  stays readable.

## Phases

> Keep checkboxes current in the same commit as the work; fill `Commit:` when a
> phase lands.

### Phase 1 — Package & type plumbing (no OS calls)

Delivers a registered `process` package whose `Process` type resolves; safe to
land alone because nothing is callable yet.

- [x] `src/builtins/process.rs` (new): `PROCESS_TYPE`, `DROP`, `PROCESS` static,
  opaque `Process` type, `is_builtin_type`, `resource_close_function`.
- [x] ~~`src/builtins/process_package.mfb` (new): companion header + `EXPORT` type
  `Process`.~~ — moot: an opaque resource handle lives ONLY in the descriptor
  `types` list, never in a companion `EXPORT TYPE` (net's `Socket`, audio's
  `AudioInput`/`AudioOutput` are all opaque and have no companion declaration).
  The companion + `augmented_project` wiring first appears in B, when the `Stream`
  enum needs it. Creating a companion here would inject a dead file every compile.
- [x] Register: `descriptor.rs` REGISTRY (`&process::PROCESS`), `mod.rs`
  (`mod process`, `is_builtin_import`, `ALL_BUILTIN_PACKAGES`,
  `qualified_builtin_type`), resolver `BUILTIN_TYPES`, spec §18 package list;
  `is_resource_type` works via the `resource.rs` entry.
- [x] `resource.rs` `BUILTIN_RESOURCES` entry (close op `process.__drop`,
  `sendable:false`, `close_may_fail:false`) + extend enumeration test.
- [~] Tag 10: `03_heap-values.md` table row added. `RESOURCE_TAG_PROCESS` const
  in `error_constants.rs` lands in Phase 3 at its first use (record stamping) — an
  unused `pub(crate) const` trips `dead_code`. `binary_repr` is moot here (see
  Corrections): `UdpSocket`(3)/`TlsSocket`(5–8)/`Audio`(9) all carry a resource tag
  yet have NO `binary_repr` handle constant and the tree is green — the wire
  handle ids are the legacy File/Socket/Listener set only. E's `.mfp` packaging
  re-verifies `Process` round-trips.
- [x] Tests: `tests/syntax/process/type_valid` (names `Process` as a param type,
  compiles) + `tests/syntax/process/type_invalid` (Integer/String passed where
  `Process` expected → `TYPE_CALL_ARGUMENT_MISMATCH`). New `tests/syntax/<pkg>/`
  layout per `.ai/compiler.md`, NOT the retired flat `tests/func_*` layout.

Acceptance: a program declaring a `Process`-typed variable type-checks and
compiles; `cargo test --bin mfb` green (3789 passed) incl. the extended
`resource.rs` enumeration test; existing goldens unchanged (only new `process`
fixtures added).
Commit: 3b6ced597

### Phase 2 — Frontend metadata for the 5 functions

Makes the 5 functions resolve with correct arity/return types (still no
codegen).

- [x] `src/builtins/process.rs`: the 6-function `PROCESS_FUNCTIONS` descriptor
  (spawn ×2, shell, pid, isRunning, waitFor, close), fully data-only. `arity`,
  `call_return_type_name`, `resolve_call` (spawn overload selection) come from
  `DefaultResolver` over the descriptor — NOT hand-written (see Corrections). A
  bespoke `expected_arguments(spawn)` names both overloads; wired into
  `mod.rs::expected_arguments`.
- [x] Extra site the plan omitted: `syntaxcheck/builtins.rs BUILTIN_ARG_MODES`
  gains a `process` row (`ArgMode::Use` — no call consumes its `Process`). Without
  it the shared checker never runs, so invalid calls collapse to `TYPE_UNKNOWN_VALUE`
  with no arity/argument-mismatch diagnostic (see Corrections).
- [x] Tests: `tests/syntax/process/{spawn,shell,pid,isRunning,waitFor,close}_invalid`
  (arity + arg-type diagnostics: `TYPE_CALL_ARITY_MISMATCH`/`TYPE_CALL_ARGUMENT_MISMATCH`)
  — the `_valid` runtime cases land in Phase 3/4. New `tests/syntax/<pkg>/` layout.

Acceptance: arity/type diagnostics for all 6 functions match golden `_invalid`
fixtures; `cargo test --bin mfb` green (3795 passed). Golden `build.log`s
re-verified via `scripts/test-accept.sh … 'syntax/process/*'` → "acceptance tests
passed (8 test(s) ran)" once the concurrent P-86 acceptance cleared the guard.
Commit: 468ccb5eb

### Phase 3 — Native Unix backend: spawn/waitFor/isRunning/pid/close + drop-reap

The mechanism. Highest blast radius; lands behind runtime tests.

- [x] `RuntimeHelper::Process` + `runtime/process_specs.rs` + `catalog.rs` wiring.
  (Commit `4f283bc3a`: `Process` variant, `is_process_runtime_call`, 8 specs
  incl. code-layer-only `spawnEnv`, `catalog_is_consistent` green.)
- [~] `src/target/shared/code/process/{mod,unix}.rs`: native lowering. DONE for
  the argv-only `spawn` + `pid`/`isRunning`/`waitFor`/`close`/`__drop` (commit
  `145c946ae`), with numeric `Vregs`. Record tail: stdin-w@32/stdout-r@40/
  stderr-r@48/reaped@56/status@64/exitcode@72; waitpid decode
  `termsig=status&0x7f`, `WEXITSTATUS=(status>>8)&0xff`, signal→-1 (via
  `bitwise_not(ZERO)` — `-1` is not a valid `move_immediate`). spawn = argv build
  from the `List OF String` entry array + 3 stdio pipes + O_CLOEXEC self-pipe +
  fork/execvp + exec-failure-over-self-pipe. `shell` DONE (`f43a36703`):
  `["/bin/sh","-c",cmd]` over the shared `emit_spawn_tail` (extracted from spawn),
  verified `shell("exit 7")`→waitFor==7. REMAINING: `spawnEnv` (child chdir +
  environment application).
- [x] Error block 7708 (`ERR_SPAWN_FAILED_*`) + `RESOURCE_TAG_PROCESS=10` +
  `02_error-codes.md` row (`4f283bc3a`); `data_objects.rs` per-package error-string
  gate + `standard_error_messages` `ErrSpawnFailed` row + raising at the self-pipe
  site / empty-argv `ErrInvalidArgument` / dropped-handle `ErrResourceClosed`
  (`145c946ae`).
- [x] libc imports + capabilities: macOS `libSystem` (`145c946ae`) AND the 3 Linux
  arches — `linux_common/plan.rs` imports (`pipe`/`fork`/`dup2`/`execvp`/`close`/
  `waitpid`/`kill`/`read`/`write`/`fcntl`/`_exit`/`__errno_location`) +
  `linux_common` `RUNTIME_CALLS` capability list (`e9ae09408`). `write` is NOT
  filtered for process (unlike net's raw-syscall write path).
- [x] `process.__drop` scope-drop wiring: works via `resource.rs`'s
  `close_function="process.__drop"` — the drop-reap fixture confirms scope exit
  emits `__drop` (no extra `builder_resource_cleanup.rs` edit was needed).
- [x] Tests: `tests/rt-behavior/process/{spawn-waitfor,spawn-fail-trap,drop-reap,
  shell-exitcode}` runtime fixtures pass on macOS (`test-accept` green, 4 tests).
  `isRunning` true→false is covered by drop-reap; the shell fixture covers
  `shell`+`waitFor`. (New-layout `tests/rt-behavior/`, NOT `tests/rt_*.rs`.)

Acceptance (runtime proof): VERIFIED on ALL FOUR targets — macOS-aarch64 (local),
linux-x86_64 musl (box 2227), linux-aarch64 glibc (box 2223), linux-riscv64 musl
(box 2229). Each: spawn `["echo","hi"]`→pid>0/waitFor==0; `sleep 30` scope-drop
SIGKILL'd+reaped (exits ~0.00s, no hang, no zombie); bogus path TRAPs
`ErrSpawnFailed` (7-708-0001, exit 255). `cargo test --bin mfb` green (3795).
Commit: e9ae09408 (macOS + all 3 Linux arches; only `spawnEnv` cwd/env remains)

### Phase 4 — riscv64 backend parity + runtime proof

- [x] Register the same libc imports for `linux_riscv64` — done via
  `linux_common` (the 3 Linux arches share `runtime_imports`/`RUNTIME_CALLS`), so
  no rv64-specific `plan.rs` edit was needed (Correction to the plan's assumption
  of a per-arch edit).
- [x] Runtime proof on the rv64 remote (`ssh -p 2229`, Alpine riscv64 musl):
  cross-compiled here, shipped, ran the Phase-3 programs — spawn-waitfor
  (pid-ok/exit 0), drop-reap (`running`, 0.00s, no zombie), fail-trap
  (`ErrSpawnFailed`, exit 255). All pass.

Acceptance: the Phase-3 runtime programs pass on the riscv64 remote. MET.
Commit: e9ae09408

## Validation Plan

- Tests: the four `rt_process_*.rs` above + `func_process_*` valid/invalid.
- Coverage check: confirm the new `process/*.rs` emission is exercised by the
  `rt_` binaries (not just compiled) — the tests run the child and read the exit
  code.
- Runtime proof: per-backend spawn/waitFor/no-zombie as in Phases 3–4.
- Doc sync: man pages `src/docs/man/builtins/process/{package,spawn,shell,pid,
  isRunning,waitFor,close}.md` (per `.ai/man_template.md`); spec error-block 7708
  in `02_error-codes.md`; `cargo test man_citations_resolve`.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual
  'process*'`; full artifact-gate is deferred to sub-plan E (one gate at
  finalization).

## Open Decisions

- **D1 — error block number.** Recommend **7708** for a small `process` block
  (`ErrSpawnFailed` + any process-specific codes) vs. reusing only existing
  `ErrIo`/`ErrResourceClosed`. Recommend a dedicated block so spawn failures are
  greppable and documented.
- **D2 — `shell` on macOS.** Recommend `bash -c` on macOS and `sh -c` on Linux
  (the API note's "whatever the default shell is"); a fixed `sh -c` everywhere is
  simpler but loses bash on macOS. Recommend per-OS default.
- **D3 — pipes created eagerly at spawn (this sub-plan) vs. lazily in B.**
  Recommend eager (create all 3 in Phase 3) so B is pure read/write wiring with
  no spawn-path change. Chosen: eager.

## Corrections

- **Phase 1 — no source companion for the opaque `Process`.** §4.1 said to create
  `process_package.mfb` with `EXPORT TYPE Process`. Evidence it is wrong: an opaque
  resource handle is declared only in the descriptor `types` list — `net`'s
  `Socket`/`Listener`/`UdpSocket` and `audio`'s `AudioInput`/`AudioOutput` are all
  `TypeKind::Opaque` and appear in NO companion `.mfb` (`rg 'EXPORT TYPE Socket'`
  → none). Declaring `EXPORT TYPE Process` in a companion would be a duplicate
  definition of the descriptor type. The companion (+ `augmented_project` wiring in
  `resolver/mod.rs` and `syntaxcheck/mod.rs`) first appears in **B**, when the
  `Stream` enum needs a source declaration. Phase 1 registers `Process` purely
  through the descriptor.
- **Phase 1 — extra registration site the plan omitted: `resolver::BUILTIN_TYPES`.**
  A bare type name in a param/return position is resolved against the resolver's
  hardcoded `BUILTIN_TYPES` slice (`src/resolver/mod.rs`), NOT the descriptor. The
  plan's "resolver BUILTIN_TYPES" bullet named it; concretely, `Process` had to be
  appended there (`builtins::process::PROCESS_TYPE`) or `AS Process` fails with
  `SYMBOL_UNKNOWN_TYPE`. Two package-list gates also had to be updated in lockstep:
  the spec §18 `is_builtin_import` sentence (`spec_section_18_package_list_matches_is_builtin_import`
  test) and the registry-size assertion in `descriptor.rs`
  (`production_registry_holds_migrated_packages`, 27→28).
- **Phase 1 — `binary_repr` tag mirror is moot.** §4.1/Phase 1 listed
  `binary_repr/{mod,sections,reader}.rs`. `sections.rs::type_id` only maps the
  legacy `File`/`Socket`/`Listener` handle ids; `UdpSocket`(tag 3),
  `TlsSocket`(5–8), and `Audio`(9) carry a runtime resource tag but have NO
  `binary_repr` handle constant, and `cargo test --bin mfb` is green. `Process`
  follows that precedent. If E's `sync-package-mfp` ever fails to round-trip a
  `Process` in an exported signature, add the handle then — flagged in E.
- **Test layout — new tree, not the retired flat one.** §Phase 1/Validation named
  `tests/func_process_type_valid/**`. Per `.ai/compiler.md` that flat layout no
  longer exists; the fixtures live at `tests/syntax/process/type_{valid,invalid}`.
- **Phase 2 — `process` is fully data-only; no hand-written `resolve_call`.** §4.2
  and the Phase 2 checkbox implied a bespoke `is_process_call`/`arity`/
  `call_return_type_name`/`resolve_call` (the net idiom). Evidence they are
  unnecessary: no `process` overload uses an argument *union* (net's `close` accepts
  `Socket|Listener|UdpSocket`), so `DefaultResolver::resolve_call`'s exact
  per-position match answers every overload, including spawn's two arities. The
  package carries `resolver: None`. Only `expected_arguments(spawn)` is
  hand-authored (two structurally different overloads the descriptor renders as the
  first form alone).
- **Phase 2 — extra site: `syntaxcheck::BUILTIN_ARG_MODES`.** The plan did not name
  it, but the shared table-driven builtin checker only runs for a package listed in
  `BUILTIN_ARG_MODES` (`builtin_arg_mode`); a missing row routes the call to the
  `Type::Unknown` fallback with NO arity/argument diagnostic. `process` was added
  as `ArgMode::Use` (it consumes no argument — `close` borrows the `Process`, and
  scope-drop's `__drop` reaps it). Verified: before the row, `process::spawn()`
  reported only `TYPE_UNKNOWN_VALUE`; after, `TYPE_CALL_ARITY_MISMATCH`.
- **`Process` is a resource → `RES`, not `LET`.** A `spawn`/`shell` result binds
  with `RES` (like `net`/`fs`), so the runtime `_valid` fixtures (Phase 3) and any
  program use `RES p = process::spawn(...)`. A `LET` binding raises
  `TYPE_RESOURCE_REQUIRES_RES`.
- **D2 resolved: `shell` uses `/bin/sh` on both platforms, not bash-on-macOS.**
  Open Decision D2 recommended `bash -c` on macOS / `sh -c` on Linux. Chosen: the
  documented simpler alternative — `["/bin/sh","-c",cmd]` everywhere. `/bin/sh`
  exists on both macOS and Linux; the 4-bucket "run this command line" abstraction
  gains nothing from bash-specific syntax, and one code path keeps the backend
  uniform. Revisit only if a concrete need for bash builtins appears.
- **Phase 3 — hand-built helper register operands must be numeric `%vN`.** A first
  lowering draft used readable names (`%vfile`, `%vstatus`). `regalloc/mod.rs`
  decodes a vreg as `value.strip_prefix("%v")?.parse()` — a NUMBER — so `%vfile`
  parses to `None`, is treated as a physical register, and `finalize_vreg_body`'s
  `find_physical_operand` guard PANICS (the zero-physical-register invariant,
  plan-34-D). Use `Vregs::next()` (or hardcode distinct `%vN`, as `net` does with
  `%v9`..`%v15`). The allocator DOES spill live vregs across every `bl`, so a value
  may live in a vreg across a libc call; only syscall-filled memory buffers
  (`pipe(int[2])`, the `waitpid` status int, the argv array) need the explicit
  `sp`-frame from `finalize_vreg_body_with_locals`. Full recipe recorded in memory
  `new-builtin-package-registration-seams`.

## Summary

The engineering risk is entirely in Phase 3: the fork/exec/3-pipe sequence and
the drop-reap cleanup. Everything before it is mechanical registration; the two
things that can go wrong at runtime — a leaked fd/zombie or an exec failure that
doesn't propagate — are pinned by dedicated `rt_` tests. No layout, ABI, or
existing-golden change.

# plan-130-A: `mfb build --debug`, the debug-feature registry, and `_mfb_debug_shutdown`

Last updated: 2026-09-12
Overall Effort: huge (>3d)
Effort: large (3h–1d)
Depends on: nothing

`mfb build --debug` (and `mfb test --debug`) produces a program that carries a
**debug report**: after the program's own output, as the very last thing
`_mfb_shutdown` does, a new runtime helper `_mfb_debug_shutdown` writes one block of
machine-readable lines to stderr. This letter lands the flag, its path from the CLI
into native codegen, the compiler-side registry that later letters plug features
into, and the report helper itself with a header and footer section, on all five
targets. It measures nothing yet; plan-130-B moves the perf timings into it,
plan-130-C adds the arena counters, plan-130-D adds peak RSS, plan-130-E moves the
canvas/app reporting hooks behind it.

Behavioral outcome for this letter: `mfb build --debug` of any console program, on
every target, exits with the same code and prints the same stdout as a normal build,
and its stderr ends with exactly one report block (`mfb.debug.begin` …
`mfb.debug.end`) — on a normal return, `EXIT PROGRAM n`, an untrapped error, and
SIGINT/SIGTERM on Unix. A build without `--debug` is byte-identical to today.

References — read these first:

- `planning/todo.md` § Memory — why the harness exists.
- `src/docs/spec/tooling/07_cli-reference.md` § `build` Flags — the flag table.
- `src/docs/spec/memory/08_program-startup.md` — entry, exit, `_mfb_shutdown`.
- `src/docs/spec/threading/09_os-integration.md` § Arena Teardown at Process Exit.
- `src/docs/spec/memory/07_runtime-helper-abi.md` — helper ABI (the perf section is
  plan-130-B's).
- `.ai/compiler.md`, `.ai/codegen-invariants.md`, `.ai/arch-abi.md`,
  `.ai/testing-gates.md`, `.ai/remote_systems.md`.

## Prerequisites

The whole plan-130 family (A–E) is gated here; letters B–E point to this table.

| Must be true | Command | Status |
|---|---|---|
| No `--debug` option exists | `git grep -n '"--debug"' -- src tests` → no matches | MET (2026-09-12, re-run by follow-plan: no matches) |
| plan-130 number unclaimed elsewhere | `git log --all --oneline --grep plan-130` → only this family | MET (2026-09-12, re-run: only `dc57f2ce5`) |
| Tree green at the base commit | `cargo test --no-fail-fast -- --skip artifact_gate_all > /tmp/p130-base.log 2>&1; echo EXIT=$?` → `EXIT=0` | MET (2026-09-12, base `b0899e778`: `EXIT=0`, 165 `test result: ok`, 0 `FAILED`) |
| Artifact gate baseline at the base commit | `scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)` | MET (2026-09-12, base `b0899e778`: `1436 tests, 1602 build(s), 2011 golden(s) checked, 0 diff(s)`) |
| Remote boxes reachable | `for p in 2223 2227 2228 2229; do ssh -o ConnectTimeout=8 -o BatchMode=yes -p $p test@127.0.0.1 true && echo $p ok; done; ssh -o ConnectTimeout=8 -o BatchMode=yes -p 2230 test@127.0.0.1 ver && echo 2230 ok` → five `ok` (2230 is Windows `cmd`: `true` is not a command there, so it is probed with `ver`) | MET (2026-09-12, re-probed after the user started the VMs: 2223/2227/2228/2229 ok; 2230 `Microsoft Windows [Version 10.0.26100.9168]` ok). Earlier the same day 2228/2230 refused connections — VMs were stopped. |

A red baseline row is recorded here with the failing test names before Phase 1 starts;
it is not this plan's to fix and its failures are not this plan's regressions.

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop.
>
> **If you stop, report the current status of *all* prerequisites** — not only the one
> that blocked you.

## 1. Goal

- `mfb build --debug` and `mfb test --debug` parse; `--debug` is listed in both help
  texts and in `07_cli-reference.md`.
- The flag reaches native codegen as a field on `NirModule`, not a process global.
- `_mfb_debug_shutdown` exists only in a `--debug` build, is called as the final call
  of `_mfb_shutdown` on both its paths (arena present, and the early-return path),
  prints its block once per process, and writes only to fd 2.
- The report format is the contract in §4.4 and is specified in the embedded spec.
- A build without `--debug` is byte-identical on every target
  (`scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)`).

### Non-goals (explicit constraints)

- No change to any non-`--debug` output: codegen, data objects, imports, goldens,
  `build.log`.
- No new MFBASIC-language surface (no `debug::` package, no source-level query of
  the flag) in this letter.
- No change to exit codes, stdout, or stderr content *before* the report block.
- `--debug` does not change the optimization level or any other flag's meaning.
- Paths that never reach `_mfb_shutdown` today (app-window close, Windows Ctrl-C,
  SIGPIPE re-raise, crashes) are not rerouted here — see Open Decisions.

## 2. Current State

**CLI.** `mfb build` arguments are parsed by `parse_build_options`
(`src/cli/build/options.rs`) into `BuildOptions` (`src/cli/build/mod.rs`);
`mfb test` has its own `parse_test_options` in the same file. Help text is
`BUILD_HELP` / `TEST_HELP` (`src/cli/help.rs`). `dispatch.rs` routes `build`/`test`.
There is no build cache or fingerprint: `build/` and `build/packages/` are rebuilt
every time (`source_packages.rs::clear_build_dir`, `build_source_dependencies`).

**Path to codegen.** `-O` travels as a process global (`optimizer::set_opt_level`),
but per-build facts that codegen reads travel on `NirModule`
(`src/target/shared/nir/mod.rs`: `target`, `build_mode`, `stdin_log_cap`), filled by
`target::shared::lower::lower_project(ir, target, packages, build_mode, stdin_log_cap)`
and read in `lower_module_for_platform` (`src/codegen/engine/builder/mod.rs`, e.g.
`module.build_mode`). This letter mirrors `stdin_log_cap`.

**Shutdown.** `lower_shutdown` (`src/codegen/os/process/process_lifecycle.rs`) is
emitted once per entry module on every backend (`lower_module_for_platform`, guarded
by `module.entry.is_some()`). It loads and zeroes `_mfb_rt_main_arena`; if already 0
it jumps to `shutdown_done` (second entry is a no-op); otherwise drains stdout, turns
the terminal off, stops graphics, and calls `_mfb_arena_destroy` unless the program
embeds a `thread.` call (`skip_entry_arena_destroy`). `shutdown_done` restores `x19`
and returns. Callers: the entry exit tail (`src/codegen/engine/function/entry.rs`,
then `emit_program_exit`) and `lower_signal_handler` (Unix console only:
`entry && !build_mode.is_app() && family != Windows`).

**Precedents this letter mirrors.**

- *Build-conditional helper + data objects that vanish when off:* plan-67's perf
  instrumentation — `_mfb_rt_perf_state` and the perf helpers exist only under one
  gate, so every other build stays byte-identical (`builder/mod.rs` data objects,
  `target/shared/plan/symbols.rs::runtime_symbols` forcing the helper symbols in).
- *Arena-free stderr writing:* `CodegenPlatform::emit_write`
  (`src/codegen/engine/types/types.rs`; macOS libc `_write`, Linux libc `write` or the
  raw syscall on x86_64, Windows `GetStdHandle`+`WriteFile`),
  `emit_write_string_object` (`entry.rs`), and perf's `emit_write_i64`
  (`src/codegen/builtins/perf/perf.rs`).
- *A writable process-global flag:* `_mfb_rt_main_arena` (`kind:"raw"`, 8 bytes,
  zeroed; `builder/mod.rs`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `BuildOptions { … }` literals needing the new field | 6 | `git grep -nE "BuildOptions \{" -- src \| grep -v "struct BuildOptions"` → `mod.rs:1180`, `options.rs:135`, `options.rs:178`, `source_packages.rs:192`, `pkg.rs:124`, `pkg.rs:418` |
| `target::shared::lower::lower_project` call sites | 12 | `git grep -nE "lower::lower_project\(\|shared::lower::lower_project\(" -- src` → linux_aarch64 1, linux_riscv64 1, linux_x86_64 1, macos_aarch64 6, win_x86_64 1, testutil 2 |
| Callers of `_mfb_shutdown` | 2 | `git grep -nE "branch_link\(SHUTDOWN_SYMBOL\)" -- src` → `entry.rs:751`, `process_lifecycle.rs:115` |
| Existing `--debug` / `MFB_DEBUG` / `-g` | 0 | `git grep -nE '"--debug"\|MFB_DEBUG\|"-g"' -- src tests` |

### Verified properties

- *`_mfb_shutdown` is reached on normal return, `EXIT PROGRAM`, untrapped error, and
  global/LINK-init failure* — all branch to `entry_exit` before `bl _mfb_shutdown`
  (`lower_program_entry`, read 2026-09-12 via research of `entry.rs`).
- *No build cache can confuse a debug build with a normal one* — no fingerprint or
  up-to-date check exists (`git grep -n fingerprint -- src/cli` hits signing only).
- *Source-dependency `.mfp` builds run no native codegen* — `write_package` does not
  call `lower_project` (`src/target/package_mfp/mod.rs`), so `--debug` need not reach
  them.
- UNVERIFIED: that a helper called after `abi::label("shutdown_done")` but before the
  `x19` restore links on every backend without a new import beyond `emit_write`'s.
  Phase 3 proves it per target.

## 3. Design Overview

Four pieces, layered:

1. **Flag** — `BuildOptions.debug: bool` → `lower_project(..., debug)` →
   `NirModule.debug: DebugOptions`.
2. **Registry** — `src/codegen/debug/mod.rs`: a `DebugFeature` trait and one static,
   ordered `DEBUG_FEATURES` list. Every hook the compiler has for debug output asks
   the registry, never `module.debug` directly, so a later letter adds a feature by
   adding one list entry.
3. **Report helper** — `_mfb_debug_shutdown`, emitted only when
   `module.debug.enabled && module.entry.is_some()`: a once-guard, the begin line,
   each feature's report section in list order, the end line.
4. **Call site** — `lower_shutdown` gains `debug_report: bool`; when true it emits
   `bl _mfb_debug_shutdown` immediately after `abi::label("shutdown_done")`, so both
   the full path and the early-return path reach it; the helper's guard makes the
   second entry print nothing.

**Correctness risk** concentrates in (4): `_mfb_shutdown` runs from a signal handler
and from app-mode workers, and a helper frame there must not clobber `x19`/the saved
arena or the exit code parked at `[arena+32]` (the entry reloads it after the call).
The report helper touches neither: it reads only its own globals.

**Design uncertainty** concentrates in "all five targets link and print": the write
seam differs per backend (Windows `WriteFile`, x86_64 raw syscall). Phase 3 is a
hello-world proof per target before any feature exists.

**Byte-identity is the gate for the OFF state only**: a non-`--debug` build must not
change a single golden. The ON state is behavior-tested (rt tests + box runs); its
`.ncode` is expected to differ and has no goldens.

### Rejected alternatives

- *A process global like `set_opt_level`.* Rejected: `-O` needed it because many
  distant optimizer sites read it; `--debug` is read only in codegen, and `NirModule`
  already carries per-build facts. A global also survives across the nested
  `build_project` calls for source dependencies.
- *Folding debug into `NativeBuildMode`.* Rejected: build mode is console vs app, an
  orthogonal axis (`--app --debug` is valid).
- *Printing from the entry exit tail (where perf prints today).* Rejected: the user
  asked for the report to be the last thing `_mfb_shutdown` runs, and the signal
  handler path never returns to the entry tail.
- *A `debug::` MFBASIC package the program calls.* Rejected: the harness observes the
  program; it must not require source changes.

## 4. Detailed Design

### 4.1 CLI

- `src/cli/build/mod.rs` `BuildOptions`: add `pub(crate) debug: bool`.
- `options.rs` `parse_build_options` and `parse_test_options`: accept exactly
  `--debug`; a repeat is an error `"mfb build accepts at most one --debug option"`
  (`"mfb test …"` for test), mirroring `--app-debug`.
- `source_packages.rs::build_source_dependency`, `pkg.rs` (both), and the test literal
  in `mod.rs`: `debug: false` (packages run no native codegen; publish never debugs).
- `help.rs`: add `--debug` under `Options:` in `BUILD_HELP` and `TEST_HELP`:
  `--debug  Build a program that prints a measurement report to stderr at exit`.

### 4.2 Transport

- `src/codegen/debug/mod.rs` (new): `pub(crate) struct DebugOptions { pub(crate) enabled: bool }`
  with `DebugOptions::OFF`. A struct, not a bool, so a later feature toggle
  (`--debug=arena,perf`) is a field, not a signature change.
- `src/target/shared/lower.rs::lower_project`: add parameter `debug: DebugOptions`;
  store `NirModule.debug`. The 12 call sites pass it (dump writers
  `write_nir`/`write_native_plan`/… pass the same value so `--debug --ncode` dumps the
  debug program).
- `target::write_executable` and the per-backend `NativeBackend::write_executable`
  gain the parameter and forward it.

### 4.3 Registry

```rust
pub(crate) trait DebugFeature: Sync {
    /// Stable section name; every report line of this feature starts with it.
    fn name(&self) -> &'static str;
    /// Whether this feature emits anything for this module on this platform.
    fn applies(&self, module: &NirModule, platform: &dyn CodegenPlatform) -> bool;
    /// Writable/readonly data objects the feature needs (counters, names).
    fn data_objects(&self, module: &NirModule, platform: &dyn CodegenPlatform) -> Vec<CodeDataObject>;
    /// Runtime helper functions the feature adds (its report section among them).
    fn code_functions(&self, module: &NirModule, platform: &dyn CodegenPlatform) -> Result<Vec<CodeFunction>, String>;
    /// Symbols of `code_functions` that must be forced into the runtime symbol set.
    fn runtime_symbols(&self) -> &'static [&'static str];
    /// Platform imports the feature's code references.
    fn imports(&self, module: &NirModule, platform: &dyn CodegenPlatform) -> Vec<PlatformImport>;
    /// Instructions emitted in the program entry after the arena is live (may be empty).
    fn emit_entry_start(&self, ctx: &mut DebugEmitCtx) -> Result<(), String>;
    /// Symbol of this feature's report section helper, called by `_mfb_debug_shutdown`.
    fn report_symbol(&self) -> Option<&'static str>;
}
pub(crate) static DEBUG_FEATURES: &[&dyn DebugFeature] = &[&CoreSection];
```

`CoreSection` is this letter's only feature: no data, no entry hook, and a report
section that writes `mfb.debug.target <target>` and `mfb.debug.build <console|app>`.
`lower_module_for_platform`, `target/shared/plan/symbols.rs` (`runtime_symbols`,
`platform_imports`) and each backend's `plan.rs` import list consult
`DEBUG_FEATURES` only when `module.debug.enabled`.

### 4.4 Report format (the contract)

Every line is `<key> <value>\n` on fd 2, keys dot-separated, values a decimal
integer or a single token without spaces. The block is bracketed:

```
mfb.debug.begin 1
mfb.debug.target macos-aarch64
mfb.debug.build console
… feature sections, in DEBUG_FEATURES order …
mfb.debug.end 1
```

The `1` after `begin`/`end` is the format version. A consumer takes the last
`mfb.debug.begin` block in stderr. Specified in a new spec page (Phase 4).

### 4.5 `_mfb_debug_shutdown`

`lower_debug_shutdown(module, platform)` in `src/codegen/debug/shutdown.rs`, a
vreg-allocated helper (`finalize_vreg_helper`): load writable global
`_mfb_rt_debug_reported`; if non-zero return; store 1; write `mfb.debug.begin 1`;
`bl` each applying feature's `report_symbol()`; write `mfb.debug.end 1`; return. No
arena access, no `x19` read. Data objects: `_mfb_rt_debug_reported` (raw 8 B zero) and
the key strings (string objects). Report sections use shared writers moved from perf
into `src/codegen/debug/write.rs`: `emit_debug_key_value(key_symbol, value_vreg)` and
`emit_debug_key_token(key_symbol, token_symbol)`, built on `emit_write`.

`lower_shutdown` gains a `debug_report: bool` parameter; the one call site in
`lower_module_for_platform` passes `module.debug.enabled`.

## Compatibility / Format Impact

- New CLI flag on `build` and `test`; nothing existing changes meaning.
- New stderr block only in `--debug` builds; its format (§4.4) is a new, versioned,
  documented contract.
- Non-`--debug` artifacts: unchanged, byte for byte.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work; `- [~]` for partial with what remains; `- [x] ~~text~~ — moot: <evidence>`
> instead of deleting; fill `Commit:` the moment a phase lands. **An unticked box
> means NOT DONE.**

### Phase 1 — the flag, parsed and carried, emitting nothing

Safe alone: the value reaches `NirModule` and nothing reads it.

- [ ] `src/cli/build/mod.rs`: `BuildOptions.debug`; `options.rs`: parse `--debug` in
      `parse_build_options` and `parse_test_options` with the at-most-once error;
      the 4 other literals pass `false`.
- [ ] `src/cli/help.rs`: `--debug` row in `BUILD_HELP` and `TEST_HELP`.
- [ ] `src/codegen/debug/mod.rs`: `DebugOptions` (module registered in the codegen tree).
- [ ] `lower_project` + `NirModule.debug`; the 12 call sites; `write_executable` and the
      dump writers forward it from `build_project`.
- [ ] Tests in `src/cli/build/mod.rs` `mod tests`: `parse_build_options_debug_sets_flag`,
      `parse_build_options_rejects_repeated_debug`, `parse_test_options_debug_sets_flag`,
      and `parse_build_options_defaults` asserts `debug == false`.

Acceptance: `cargo test --bin mfb parse_` green with the new tests;
`scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)`;
`target/release/mfb build --debug examples/hello_world` succeeds.
Commit: —

### Phase 2 — registry and `_mfb_debug_shutdown` on macOS

- [ ] `src/codegen/debug/mod.rs`: `DebugFeature`, `DEBUG_FEATURES = [&CoreSection]`,
      `DebugEmitCtx`.
- [ ] `src/codegen/debug/write.rs`: `emit_debug_key_value`, `emit_debug_key_token` on
      `platform.emit_write` (fd 2), no arena symbol referenced.
- [ ] `src/codegen/debug/shutdown.rs`: `lower_debug_shutdown` + its data objects.
- [ ] `lower_module_for_platform`: when `module.debug.enabled && module.entry.is_some()`
      push the helper, each feature's `code_functions`/`data_objects`; pass
      `debug_report` to `lower_shutdown`.
- [ ] `process_lifecycle.rs::lower_shutdown`: `bl _mfb_debug_shutdown` right after
      `abi::label(done)` when `debug_report`.
- [ ] `target/shared/plan/symbols.rs`: force `_mfb_debug_shutdown` and each feature's
      `runtime_symbols()` into the set, and add feature imports, only when enabled.
- [ ] Unit test `src/codegen/debug/tests.rs`: `debug_helpers_reference_no_arena_symbol`
      (mirrors `perf.rs`'s `no_perf_body_calls_an_arena_helper`) and
      `shutdown_calls_debug_report_after_the_done_label`.
- [ ] Runtime test `tests/runtime/rt_debug_report.rs` (macOS host): builds a program
      with `--debug`; asserts stdout equals the normal build's, exit code equal, stderr's
      last lines are exactly the §4.4 block with `target macos-aarch64`, for four
      programs — plain return, `EXIT PROGRAM 3`, untrapped error, and SIGTERM sent
      mid-sleep — and that a normal build's stderr contains no `mfb.debug.`.

Acceptance: `cargo test --release --test rt_debug_report` green on macOS; artifact
gate `0 diff(s)`.
Commit: —

### Phase 3 — the other four targets, runtime-proven

- [ ] linux-aarch64: cross-build the Phase 2 programs, run the `-glibc.out` on 2223;
      record the stderr tail here.
- [ ] linux-x86_64: run glibc on 2228 and musl on 2227 (raw-syscall write path).
- [ ] linux-riscv64: run on 2229.
- [ ] windows-x86_64: run on 2230 via a CRLF `.bat` (see memory
      `windows-box-has-no-test-script`); the SIGTERM case is not applicable (no signal
      handler on Windows) — record plain return, `EXIT PROGRAM 3`, untrapped error.
- [ ] `--app --debug` on macOS (`scripts/test-macapp.sh`-style headless run with
      `MFB_MACAPP_HEADLESS=1`): the worker's normal finish prints the block.
- [ ] Extend `tests/runtime/rt_debug_report.rs` with codegen-inspection cases for the
      four non-host targets: the `.ncode` of a `--debug` build contains a call to
      `_mfb_debug_shutdown` inside `_mfb_shutdown` after `shutdown_done`, and a
      normal build contains no `_mfb_debug` symbol.

Acceptance: every row above has its recorded stderr tail in the Corrections-adjacent
proof log below with the exact §4.4 block; artifact gate `0 diff(s)`.
Commit: —

### Phase 4 — spec, docs, and the full gate

- [ ] `src/docs/spec/tooling/07_cli-reference.md` § `build` Flags: `--debug` row and
      prose; the `test` usage row.
- [ ] New spec topic `src/docs/spec/tooling/09_debug-report.md` (a new topic is
      `NN_slug.md` beside the package's `spec.md`, auto-discovered by prefix —
      `.ai/specifications.md` § Adding a topic) documenting §4.4, the call site, and
      the paths that do not reach `_mfb_shutdown`. Later letters add their sections to
      this page.
- [ ] `src/docs/spec/memory/08_program-startup.md`: `_mfb_shutdown`'s last call.
- [ ] Full suite `cargo test --no-fail-fast -- --skip artifact_gate_all`,
      `scripts/artifact-gate.sh target/release/mfb all`,
      `scripts/test-accept.sh target/release/mfb target/accept-actual`.

Acceptance: all three commands green (acceptance: no mismatch beyond the recorded
baseline); `cargo test -p mfb --bins citations_resolve` green.
Commit: —

## Validation Plan

- Tests: CLI parse unit tests (`src/cli/build/mod.rs`); codegen unit tests
  (`src/codegen/debug/tests.rs`); runtime `tests/runtime/rt_debug_report.rs`.
- Coverage check: the OFF state is covered by every existing golden (artifact gate);
  the ON state is covered only by `rt_debug_report.rs` and the box runs — no golden
  builds `--debug`, by design.
- Runtime proof: Phase 3 box runs on all five targets.
- Doc sync: `07_cli-reference.md`, new debug-report spec page, `08_program-startup.md`.
- Acceptance: the Phase 4 commands.

## Open Decisions

- **Report on paths that skip `_mfb_shutdown`** (app-window close on macOS/GTK/Windows,
  Windows Ctrl-C, SIGPIPE re-raise, crashes) — recommended: a follow-up sub-plan after
  E that routes app-window close and Windows console Ctrl-C through `_mfb_shutdown`
  (they also skip the stdout drain and terminal restore today); crashes stay out of
  scope. Alternative: accept the gap and document it (§ new spec page).
- **Per-feature selection** (`--debug=arena,perf`) — recommended: not now; `DebugOptions`
  is a struct so it can be added without touching call sites again.

## Corrections

## Summary

The risk is the call site inside `_mfb_shutdown` on five backends and the signal path,
not the flag. Everything is gated so an ordinary build cannot change; the ON state is
proven by running it on every target before any measurement feature depends on it.

# plan-130-E: move the canvas/app reporting hooks behind `--debug`

Last updated: 2026-09-12
Effort: large (3h–1d)
Depends on: plan-130-D

Three environment variables make every ordinary canvas/app program write diagnostic
output: `MFB_CANVAS_STATS` (a per-frame counters line), `MFB_CANVAS_DUMP` (the raw
frame bytes), and `MFB_WINAPP_DUMP` (the Windows app transcript readback). They are
reporting hooks, not program behavior, so they move behind `--debug`: a normal build
no longer contains the code that reads them. The control switches that tests use to
*drive* a program — `MFB_*APP_HEADLESS`, `MFB_CANVAS_SYNC`, `MFB_CANVAS_GPU`,
`MFB_CANVAS_RESIZE_W/H`, `MFB_WINAPP_INPUT` — stay in normal builds (the user's
decision, 2026-09-12).

Behavioral outcome: a `--debug` canvas/app build honours the three variables exactly as
today; a normal build ignores them (setting `MFB_CANVAS_STATS` writes no file); every
test and script that sets them builds with `--debug` and passes.

References: plan-130-A (registry, `DebugOptions`); `.ai/canvas-threading.md`;
`.ai/testing-gates.md` § Canvas reference images; memory note
`never-add-lowering-variants` (registry API changes need the user's approval).

## Prerequisites

See plan-130-A § Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-130-D complete | `ls planning/completed/plan-130-D-*` → one match | MET (2026-09-12) |
| The helper-source gating mechanism (Open Decision 1) is decided by the user and recorded in this file | the Open Decisions entry reads `DECIDED:` | MET (2026-09-12: the user chose option 1; `grep -n "DECIDED:" planning/plan-130-E-*.md` → the recorded decision) |
| Canvas suites green at the base | `cargo test --release --test 'rt_canvas_*'` → all pass | MET (2026-09-12, base `b0899e778`, run with `--no-fail-fast`: `EXIT=0`, 9 binaries, 153 passed, 0 failed) |

## 1. Goal

- `__canvas_writeStats` and the `MFB_CANVAS_DUMP` read in `__canvas_presentSurface`
  exist only in `--debug` builds.
- The Windows app runtime's `MFB_WINAPP_DUMP` data object and its reader exist only in
  `--debug` builds.
- Every consumer listed in §2 builds with `--debug` and is green.
- A normal canvas build: setting `MFB_CANVAS_STATS`/`MFB_CANVAS_DUMP` produces no file
  (new negative test).

### Non-goals

- The HEADLESS/SYNC/GPU/RESIZE/INPUT switches do not move.
- The stats line format and the dump format do not change.
- No change to what a canvas program draws.

## 2. Current State

- `MFB_CANVAS_STATS`: read in `__canvas_writeStats`
  (`src/codegen/builtins/canvas/helper_surface.rs`, MFBASIC helper source), called from
  `__canvas_presentSurface` and the skipped-frame path in `helper_render.rs`.
- `MFB_CANVAS_DUMP`: read in `__canvas_presentSurface` (`helper_surface.rs`).
- `MFB_WINAPP_DUMP`: `DUMP_ENV_SYM` data object in `src/target/win_x86_64/app/mod.rs`
  (UTF-16 name) and its reader in the same file.
- Canvas helpers are registered with `RegistryHelper::always(name, BODY)`
  (`git grep -n "RegistryHelper::always" -- src/codegen/builtins/canvas`); **no
  mechanism exists for build-conditional helper source** (measured 2026-09-12: no
  `build_mode`/debug switch in `src/codegen/builtins/canvas`; `AbiCtx.build_mode` in
  `src/codegen/registry/mod.rs` reaches native `abi_*` lowerings only, not MFBASIC
  helper text).

### Measured populations

| What | Count | Command |
|---|---|---|
| Test files setting `MFB_CANVAS_STATS` | 7 | `git grep -l MFB_CANVAS_STATS -- tests` → rt_canvas_{damage,font,golden,graphics_thread,group_ownership,metal,rasteriser} |
| Test files setting `MFB_CANVAS_DUMP` | 5 | `git grep -l MFB_CANVAS_DUMP -- tests` → rt_canvas_{damage,font,golden,metal,rasteriser} |
| `common::build_app` calls in `tests/canvas` | 21 | `git grep -hoE "common::build_app" -- tests/canvas \| wc -l` |
| Scripts setting the moved variables | 2 | `test-canvas-vulkan.sh` (builds at lines 250, 544); `test-winapp.sh` (build at 170 feeds runs at 177, 240–241, 336–337); `test-macapp.sh` mentions `MFB_CANVAS_DUMP` only in a comment (line 475) |
| Tests/scripts using `MFB_WINAPP_DUMP` | 0 | `git grep -l MFB_WINAPP_DUMP -- tests scripts` → no matches |
| Docs naming the moved variables | 2 | `.ai/canvas-threading.md`, `.ai/testing-gates.md` |

### Verified properties

- VERIFIED (2026-09-12) — no *non-test* user program depends on `MFB_CANVAS_STATS`/`DUMP`/
  `MFB_WINAPP_DUMP`: grepping `src` and `examples` for the three names finds nothing under
  `examples/`; under `src` only the two hook readers (`helper_surface.rs`), the Windows app's
  `DUMP_ENV_SYM` and its reader, and comments.

## 3. Design Overview

`CanvasReportFeature: DebugFeature` owns the three hooks. Its mechanics depend on Open
Decision 1. Under the recommended option:

- `RegistryHelper` gains a debug-aware form: `RegistryHelper::debug_split(name,
  normal_body, debug_body)`. Helper emission (where `RegistryHelper` bodies are added
  to the program) picks `debug_body` when `module.debug.enabled`.
- `__canvas_presentSurface`'s normal body omits `__canvas_writeStats()` and the dump
  read; its debug body is today's text. `__canvas_writeStats` becomes debug-only.
- The skipped-frame call site in `helper_render.rs` gets the same split.
- Windows app: `DUMP_ENV_SYM` and its reader are emitted only when `module.debug.enabled`.
- `tests/common/mod.rs`: `build_app(project, name)` keeps its signature and gains a
  sibling `build_app_debug(project, name)` that adds `--debug`; the suites that set a
  moved variable switch to it.

**Risk:** the canvas goldens and the GPU oracle comparisons — a missed `--debug` in a
suite turns "no stats file" into a confusing test failure. Every consumer in §2 is a
task.

## Phases

### Phase 1 — mechanism (after the decision)

- [x] Implement the decided mechanism (Open Decision 1) with a unit test that a
      debug-split helper emits `normal_body` without `--debug` and `debug_body` with it.
      `RegistryHelper::debug_split` → two helpers gated `HelperGate::NormalBuildOnly` /
      `DebugBuildOnly`, rendered inline by `RegistryPackage::get_mfb_for(debug)`; the flag
      rides `Registry::augment_project(ast, debug)` ← `resolver::augment_project(ast, debug)` ←
      `options.debug` in `cli/build`; every other caller passes `false`. Unit test
      `a_debug_split_helper_renders_the_half_matching_the_build`.
- [x] Grep `src/codegen/builtins/canvas/**` descriptors and `examples/` for the three
      variables; record the result under Verified properties.

Acceptance: unit test green; artifact gate `0 diff(s)`.
Measured: `cargo test --bin mfb -- codegen::registry codegen::builtins::canvas` green (with
Phase 2's split in place); artifact gate `1437 tests, 1603 build(s), 2013 golden(s) checked,
0 diff(s)` — no golden holds canvas source.
Commit: —

### Phase 2 — canvas hooks

- [x] Split `__canvas_presentSurface` and the skipped-frame path; make
      `__canvas_writeStats` debug-only.
      `helper_surface.rs`: `PRESENT_SURFACE` (blit only) / `PRESENT_SURFACE_DEBUG` (today's text,
      with `__canvas_writeStats`, `__canvas_drawsText`, `__canvas_damageText`).
      `helper_render.rs`: `RENDER_LOOP` / `RENDER_LOOP_DEBUG` built with `concat!` from two
      literal macros split at the skipped-frame line, so the long body exists once. Unit tests
      `reassembled_source_parses` (both halves) and `the_normal_source_has_no_reporting_hook`.
- [x] `tests/common/mod.rs::build_app_debug`; switch the 7 STATS / 5 DUMP suites
      (union: 7 files) to it where they set a moved variable.
      11 build sites switched, each checked to feed a run that sets a moved variable:
      damage 1, group_ownership 2, graphics_thread 1, golden 1, font 3, rasteriser 3, metal 1;
      the 6 builds whose runs set neither stay normal builds.
- [x] New `tests/canvas/rt_canvas_debug_hooks.rs`: normal build + `MFB_CANVAS_STATS`
      set → no file; `--debug` build → one stats line per frame.
      Both variables set in both cases: the normal build writes neither file; the `--debug`
      build writes one `frames=1` line and a non-empty RGBA dump.
- [x] `scripts/test-canvas-vulkan.sh` lines 250 and 544: add `--debug`.

Acceptance: `cargo test --release --test 'rt_canvas_*'` green on macOS;
`scripts/test-canvas-vulkan.sh target/release/mfb --box 2228 --libc glibc` and
`--box 2227 --libc musl` report `ok`.
Measured: `cargo test --release --no-fail-fast --test 'rt_canvas_*'` — damage 6, debug_hooks 2,
font 19, golden 19, graphics_thread 8 (2 ignored), group_ownership 11, image_decode 15, metal 7,
present_deep_copy 8, rasteriser 60 (2 ignored): all passed. 2228 glibc: `EXIT=0`, `canvas Vulkan
runtime tests passed` (Vulkan render matches the software oracle, groups match
`tests/golden/canvas/groups.png`). 2227 musl: `EXIT=0` with `skip: box 2227 built no Vulkan
device (loader present, no usable ICD)` — the `--debug` program ran and wrote its stats line
(`vulkanReady=FALSE`); the box has no usable Vulkan driver (see Corrections).
Commit: —

### Phase 3 — Windows app hook and docs

- [x] `win_x86_64/app/mod.rs`: `DUMP_ENV_SYM` + reader behind `module.debug.enabled`.
      `AppEntrySpec.debug_hooks` (from `module.debug.enabled`) gates the reader in `emit_main`;
      `app_mode_data_objects(project, debug_hooks)` emits `MFB_WINAPP_DUMP` and
      `_mfb_winapp_testbuf` only then. Unit test `the_dump_readback_exists_only_in_a_debug_build`.
- [x] `scripts/test-winapp.sh` line 170: add `--debug`; run it against 2230.
      `scripts/test-winapp.sh target/release/mfb` → `EXIT=0`, 32 `ok` lines, `windows app-mode,
      canvas and Vulkan runtime tests passed`.
- [x] `.ai/canvas-threading.md`, `.ai/testing-gates.md`: the variables need a `--debug`
      build.
- [x] Debug-report spec page: a "hooks" section listing the three variables.
      `09_debug-report.md` § Reporting hooks; `cargo test --bin mfb -- docs::spec` green.
- [ ] Full suite + artifact gate + test-accept as in plan-130-A Phase 4.

Acceptance: `scripts/test-winapp.sh target/release/mfb` passes on 2230; the three
suites green.
Commit: —

## Validation Plan

- Tests: `rt_canvas_debug_hooks.rs` (new), the 7 updated canvas suites, the
  debug-split unit test.
- Runtime proof: Vulkan script on 2227/2228, winapp script on 2230, macOS canvas suites.
- Doc sync: `.ai/canvas-threading.md`, `.ai/testing-gates.md`, debug-report spec page.

## Open Decisions

1. **How MFBASIC helper source becomes build-conditional** — the registry has no such
   mechanism, and adding one widens the registry API, which needs the user's approval
   (memory `never-add-lowering-variants`). Options:
   - (recommended) `RegistryHelper::debug_split(name, normal_body, debug_body)`: explicit,
     keeps the normal body free of any debug text, localized to helper emission.
   - A compiler-provided constant the helper source tests (`IF __MFB_DEBUG THEN …`),
     folded away in normal builds: no registry change, but adds language surface and
     depends on constant folding removing the dead branch byte-for-byte.
   - A native `canvas` member that returns the variable only in debug builds
     (`abi_function` reading `AbiCtx`): no registry change, but the call itself stays in
     normal builds, so normal canvas codegen changes.
   Record the user's choice here as `DECIDED: <option> (<date>)` before Phase 1.

   DECIDED: `RegistryHelper::debug_split(name, normal_body, debug_body)` (2026-09-12). The user's
   stated goal is that the hooks' code is ABSENT from normal builds, which rules out the native
   member (code stays, switched off) and leaves the constant option dependent on unverified
   dead-branch + dead-function removal. Research at the decision: helpers are pasted at the
   SOURCE stage by `Registry::augment_project(&self, ast)`, which receives no build options, so
   the flag must be threaded into it and its callers
   (`git grep -n "augment_project(" -- src` → cli/build, resolver ×2, ir/lower ×2, audit,
   ir/shape ×2, testutil ×2).

## Corrections

- **Phase 1 — `debug_split` is two helpers, not a new field.** A `debug_body` field on
  `RegistryHelper` would have broken its 20 struct-literal constructions (crypto, strings,
  term helpers). Two new inline gates carry the halves instead; every `HelperGate` match lists
  all variants, so a future gate cannot fall through silently.
- **Phase 1 — resolution sees the normal sources.** `resolver::resolve_project` augments to
  check the program, not a build flavor, so it passes `false`; only the build's own
  augmentation (the one lowering consumes) passes `options.debug`.
- **Phase 2 — the box scripts had rotted before this plan.** Their embedded programs no longer
  built: `canvas::Color`/`rgb`/`rgba` (gone since `0b3fc656f`, plan-122-D), `canvas::fontRef`
  (a `Text` holds its `RES canvas::Font` directly), bare `Mode`/`DrawItem`/`Rectangle`/
  `Size`/`TermSize`, and `tests/rt_canvas_golden.rs` (now `tests/canvas/`). All fixed; every
  embedded program in `test-canvas-vulkan.sh`, `test-winapp.sh` and `test-macapp.sh` (22 quoted
  heredocs, 3 unquoted with `$proj` substituted) compiles with the current compiler.
  `test-macapp.sh` was compiled, not run: its GUI cases drive the desktop.
- **Phase 2 — 2227 has no usable Vulkan ICD today.** The acceptance's `ok` on 2227 is a `skip`
  from the script's own device probe, not a failure of this change: the `--debug` program ran
  headless and wrote its stats line. The Vulkan path is proven on 2228.
- **Phase 2 — the normal-source check looks for code, not words.** Comments in the canvas
  source still name `MFB_CANVAS_STATS`; the unit test asserts the quoted variable names and
  `__canvas_writeStats` are absent.
- **Phase 3 — three Windows app ncode goldens change, by exactly the removed hook.** The gate
  reported `3 diff(s)`: `syntax/app/macos-app-mode-{io,plumbing,term}` `.windows-x86_64.app.ncodesum`.
  A compiler built from `HEAD` (`bddb8632c`, `git archive` + `cargo build --release`) reproduces
  the committed hash for `macos_app_mode_io`; the new compiler's dump differs only in `_main`
  (210 → 179 instructions; relocations to `GetStdHandle`, `SendMessageW`, `WriteFile`,
  `_mfb_winapp_dump_env`, `_mfb_winapp_testbuf` gone) and in `dataObjects` (exactly
  `_mfb_winapp_dump_env` and `_mfb_winapp_testbuf` gone; every shared object identical). The
  three hashes were regenerated with `scripts/sync-goldens.sh`.

## Summary

The work is mostly consumer bookkeeping — 7 suites, 2 scripts, 2 docs — around one
mechanism decision the user has to make before this letter starts.

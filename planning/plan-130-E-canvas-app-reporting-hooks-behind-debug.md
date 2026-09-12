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
| plan-130-D complete | `ls planning/completed/plan-130-D-*` → one match | NOT MET |
| The helper-source gating mechanism (Open Decision 1) is decided by the user and recorded in this file | the Open Decisions entry reads `DECIDED:` | NOT MET |
| Canvas suites green at the base | `cargo test --release --test 'rt_canvas_*'` → all pass | UNMEASURED |

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

- UNVERIFIED — that no *non-test* user program depends on `MFB_CANVAS_STATS`/`DUMP`
  (they are undocumented in `mfb man canvas`); Phase 1 greps the man descriptors and
  `examples/`.

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

- [ ] Implement the decided mechanism (Open Decision 1) with a unit test that a
      debug-split helper emits `normal_body` without `--debug` and `debug_body` with it.
- [ ] Grep `src/codegen/builtins/canvas/**` descriptors and `examples/` for the three
      variables; record the result under Verified properties.

Acceptance: unit test green; artifact gate `0 diff(s)`.
Commit: —

### Phase 2 — canvas hooks

- [ ] Split `__canvas_presentSurface` and the skipped-frame path; make
      `__canvas_writeStats` debug-only.
- [ ] `tests/common/mod.rs::build_app_debug`; switch the 7 STATS / 5 DUMP suites
      (union: 7 files) to it where they set a moved variable.
- [ ] New `tests/canvas/rt_canvas_debug_hooks.rs`: normal build + `MFB_CANVAS_STATS`
      set → no file; `--debug` build → one stats line per frame.
- [ ] `scripts/test-canvas-vulkan.sh` lines 250 and 544: add `--debug`.

Acceptance: `cargo test --release --test 'rt_canvas_*'` green on macOS;
`scripts/test-canvas-vulkan.sh target/release/mfb --box 2228 --libc glibc` and
`--box 2227 --libc musl` report `ok`.
Commit: —

### Phase 3 — Windows app hook and docs

- [ ] `win_x86_64/app/mod.rs`: `DUMP_ENV_SYM` + reader behind `module.debug.enabled`.
- [ ] `scripts/test-winapp.sh` line 170: add `--debug`; run it against 2230.
- [ ] `.ai/canvas-threading.md`, `.ai/testing-gates.md`: the variables need a `--debug`
      build.
- [ ] Debug-report spec page: a "hooks" section listing the three variables.
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

## Corrections

## Summary

The work is mostly consumer bookkeeping — 7 suites, 2 scripts, 2 docs — around one
mechanism decision the user has to make before this letter starts.

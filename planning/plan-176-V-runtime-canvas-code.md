# plan-176-V: Runtime canvas and thread code

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-U

V splits the last four over-budget code files of plan-176, all under `src/codegen/runtime/`.
They are the canvas Vulkan backend (`canvas/vulkan.rs`, 7300 code LOC, the largest file in the
tree), its Windows swapchain child module (`canvas/vulkan_present.rs`, 1848), the canvas runtime
root (`canvas/mod.rs`, 2260), and the thread runtime helpers
(`thread/runtime_helpers_thread.rs`, 1362). Four phases are pure code motion, and each turns a
file into a directory module whose `mod.rs` keeps the surface. `vulkan_present.rs` moves under
`vulkan/present/` as an ordinary `mod present;` once `vulkan.rs` becomes `vulkan/mod.rs`.
`emit_vulkan_draw_scene` is 1060 code LOC on its own. The last phase decomposes it by
extract-method refactoring (§4.5). Every phase is gated by byte-identical codegen. V is last
because it has the largest blast radius: the canvas runtime is emitted into every canvas program
on every target, and the only golden that sees it is one app-mode fixture.

Single outcome: after V lands, `cargo test --test file_size_budget` prints no row for any file
under `src/codegen/runtime/canvas/` or `src/codegen/runtime/thread/`, and
`tests/guards/file_size_ceilings.txt` has no `ceiling <code|inline|test>` row for any of V's four
files. There are no exemptions (rule revised by the owner, 2026-10-18; only `GENERATED` files are
exempt automatically). V's Validation Plan is also the final gate of all of plan-176: the full
workspace suite, the full artifact gate, the canvas GPU box checks, and an empty ceiling file
(no `ceiling` rows of any kind).

References:

- `planning/plan-176-A-file-size-guard.md`: the rule, §Decisions, the guard, and the
  "Splitting an over-budget file (plan-176)" conventions in `.ai/build-tooling.md`.
- `.ai/canvas-threading.md`, required reading.
  - §1 and §2: arena state is per thread, and nothing crosses an arena.
  - §10: the renderer branch, and the Windows direct present at line 585, which names
    `runtime/canvas/vulkan_present.rs`.
  - §10.1: the builder thread has no arena, the swapchain stays on the graphics thread, the
    ready-word order lives in `vulkan_handoff.rs`, and `grep -rn GRAPHICS_OFFSET_VULKAN
    src/target` finds nothing.
- `.ai/codegen-invariants.md` and `.ai/compiler.md`: allocation order. The extract phase must
  not reorder any `allocate_stack_object()` / `label()` / `emit` call.
- `.ai/arch-abi.md` line 70: cites `runtime/canvas/vulkan.rs:emit_int_arg_zero`.
- `.ai/testing-gates.md`:
  - The artifact gate does NOT cover canvas through `tests/byte-identity/`. The one canvas
    golden is `tests/syntax/app/app-mouse-surface`, a change sentinel for both GPU emitters.
  - The GPU scripts are the real instruments.
  - Never run two gates at once.
- `.ai/build-tooling.md`: rustfmt policy (`rustfmt --edition 2021 <files>`).
- `.ai/remote_systems.md`: boxes 2228 (x86_64 glibc, lavapipe), 2226 (aarch64 glibc,
  lavapipe) and 206 (Windows, the only real GPU).
- Spec topics whose citations move:
  - `app/06_canvas.md`: 14 `vulkan.rs`, 4 `vulkan_present.rs` and 44 `canvas/mod.rs`
    citations.
  - `tooling/01_project-manifest.md` and `tooling/09_debug-report.md`: 1 and 2 `canvas/mod.rs`
    citations.
  - The `runtime_helpers_thread.rs` citations: `threading/06_thread-runtime-helpers.md`,
    `07_control-block.md`, `08_queue-semantics.md`, `09_os-integration.md`,
    `11_error-propagation.md`, `language/08_error-model.md` and
    `language/15_resource-management.md`.
  - Commands are in §2.
- Split maps used to write this plan: `/tmp/loc/maps/V.md` (layouts) and
  `/tmp/loc/maps/V-extract.md` (the `emit_vulkan_draw_scene` extraction, which supersedes
  V.md's draw-scene entry).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-U complete and archived | `ls planning/plan-176-U-* planning/completed/plan-176-U-*` → only `planning/completed/…` | MET (2026-10-20, `ls planning/completed/` → `plan-176-U-engine-code.md`; none in `planning/`) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | MET (2026-10-20, passes) |
| plan-176-J moved `vulkan.rs`'s inline tests to the one test-only file `vulkan/tests/mod.rs` and left `assert_layout` in `vulkan.rs` | `grep -c '^mod tests {' src/codegen/runtime/canvas/vulkan.rs` → `0`; `ls src/codegen/runtime/canvas/vulkan/tests/mod.rs` → exists; `grep -c 'fn assert_layout' src/codegen/runtime/canvas/vulkan.rs` → `1` | MET (2026-10-20: `0`, exists, `1`) |
| V's files have no uncommitted changes | `git status --porcelain -- src/codegen/runtime/canvas src/codegen/runtime/thread src/docs/spec/app/06_canvas.md src/docs/spec/tooling/01_project-manifest.md src/docs/spec/tooling/09_debug-report.md src/docs/spec/threading src/docs/spec/language/08_error-model.md src/docs/spec/language/15_resource-management.md .ai/canvas-threading.md .ai/arch-abi.md src/target/macos_aarch64/app src/target/win_x86_64/app src/codegen/builtins/canvas tests/canvas tests/codegen/codegen_canvas_vulkan_off_frame_path.rs tests/guards/file_size_ceilings.txt tools/loc-census` → empty | MET (2026-10-20, empty output) |

`assert_layout` must stay in the host because `vulkan_present.rs`'s own test module calls it 10
times (`grep -c 'assert_layout(' src/codegen/runtime/canvas/vulkan_present.rs` → 10) through
`use super::*`. A `vulkan/tests/` child module is not reachable from `present` that way.

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify — a
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you. The reader needs to know how far off the gate is,
> not just which row you hit first.

## 1. Goal

- Each of the four files becomes a directory module. `canvas/mod.rs` keeps its path and gains
  sibling parts. Every resulting file is ≤ 900 code LOC and ≤ 250 inline test LOC by
  `python3 -I tools/loc-census/census.py --file <path>`.
- No function under `src/codegen/runtime/` is over 1000 code LOC. `emit_vulkan_draw_scene` is
  decomposed exactly as §4.5 specifies. The largest function afterwards is `emit_ensure_swapchain`
  at 844 code LOC (`python3 -I /tmp/loc/span.py src/codegen/runtime/canvas/vulkan_present.rs 521
  1418` → 844); that function is not split.
- `.ncode` is byte-identical on every target before and after each phase, as is every other
  artifact-gate dump kind.
- Every `[[…]]` spec citation, `.ai/` reference and code or shader comment that names one of the
  four old paths now names the file that holds the cited symbol.
- `vulkan_present.rs` lives at `src/codegen/runtime/canvas/vulkan/present/` and is declared by a
  plain `mod present;` in `vulkan/mod.rs`. No `#[path]` attribute remains in
  `src/codegen/runtime/canvas/` (`grep -rn '#\[path' src/codegen/runtime/canvas` → empty).
- plan-176 as a whole is closed: the guard reports zero rows, and the ceiling file holds no
  `ceiling` row of any kind (Validation Plan).

### Non-goals (explicit constraints)

- No behavior change and no change to emitted code on any target.
- No logic edit except the decided `emit_vulkan_draw_scene` extraction (§4.5). The pure-motion
  phases move text verbatim. The only other edits are:
  - visibility (`pub(super)`, `pub(in crate::codegen::runtime::canvas::vulkan)`);
  - `use` lines, `mod` declarations and re-exports;
  - the two `include_bytes!` paths in §4.3;
  - the removal of the `#[path]` attribute.
- No reordering of emitted instructions. In particular the ready-word store-release/load-acquire
  order of §10.1 is untouched, and `vulkan_handoff.rs` is not edited.
- Thread ownership stays visible in the layout, and nothing leaves `src/codegen/runtime/canvas/`.
  `grep -rn GRAPHICS_OFFSET_VULKAN src/target` stays empty, and every `GRAPHICS_OFFSET_*` word
  stays in canvas (`state_layout.rs`). Ownership by file:
  - builder thread: `vulkan/build.rs`, `descriptors.rs`, `pipeline.rs`;
  - graphics thread: `target.rs`, `draw_scene/`, the uploads and `present/`;
  - shared call plumbing: `calls.rs`, `memory.rs`.
- No crate path that callers use changes. Each of these resolves unchanged through the
  re-exports in §4:
  - `crate::codegen::runtime::canvas::X`;
  - `canvas::vulkan::{emit_vulkan_build, emit_vulkan_ready, emit_vulkan_draw_scene,
    data_objects, has_vulkan_backend, CANVAS_CHILD_SYMBOL, WM_APP_CANVAS_SHOW}`;
  - `crate::codegen::runtime::thread::X`.
- No test fn renamed, no test module renamed, no `[[test]]` target renamed. The unit-test count
  of the `mfb` binary is unchanged by every phase.
- The `shaders/` directory does not move (`scripts/regen-spirv.sh` and `tools/canvas-curve-shader`
  name it). No `.spv` is regenerated.
- The stems `vulkan_handoff` and `vulkan_present` are not reused for new files.
- `bugs/*.md` and `planning/*.md` that name the old paths are historical records and are not
  edited.

## 2. Current State

### Files (as of 17a9b5799; unchanged at a3c95fc07 except where noted)

| File | Kind | Code LOC | Inline test LOC | `#[test]` | Lines (`wc -l`) |
|---|---|---|---|---|---|
| `src/codegen/runtime/canvas/vulkan.rs` | code | 7300 | 748 at planning; 31 after plan-176-J (`SPIRV_MAGIC` 2 + `assert_layout` 29) | 8, all in J's `vulkan/tests/mod.rs` | 9862 |
| `src/codegen/runtime/canvas/mod.rs` | code | 2260 | 201 (inline `mod tests` 178 by `span.py … 4056 4284`; `signal_redraw_arguments` 21 and the two-line cfg(test) `GEO_KIND_GROUP` at 1066–1067 make up the other 23) | 6 | 4284 |
| `src/codegen/runtime/canvas/vulkan_present.rs` | code | 1848 | 164 (inline `mod tests`, lines 2063–2233) | 2 | 2233 |
| `src/codegen/runtime/thread/runtime_helpers_thread.rs` | code | 1362 at a3c95fc07 (1344 at 17a9b5799) | 0 | 0 | 1664 |

LOC sources:
- vulkan, mod and present: `/tmp/loc/groups.json` (planning census
  `python3 -I /tmp/loc/census.py "$PWD"`). After A, the command is
  `python3 -I tools/loc-census/census.py --file <path>`.
- thread at HEAD: `python3 -I /tmp/loc/span.py src/codegen/runtime/thread/runtime_helpers_thread.rs
  1 1664` → 1362.

Drift since the maps:
- `git diff --stat 17a9b5799 HEAD -- src/codegen/runtime src/docs/spec/app/06_canvas.md
  .ai/canvas-threading.md` shows one change. `runtime_helpers_thread.rs` gained 21 lines in
  `lower_thread_finish` (`e9620387b`, the `program_shutdown` error report).
- Its line ranges below are therefore given at a3c95fc07. All other ranges are "as of
  17a9b5799" and still hold at a3c95fc07.
- After plan-176-J, `vulkan.rs` lines 1–9046 are unchanged. J replaces lines 9047–9862 with
  `#[cfg(test)] mod tests;`.
- Item and test NAMES are authoritative if lines drift.

### Measured populations

| What | Count | Command |
|---|---|---|
| Spec citations of `vulkan.rs` | 14, all in `app/06_canvas.md` | `grep -rnoE "\[\[src/codegen/runtime/canvas/vulkan\.rs:[^]]+\]\]" src/docs/spec \| wc -l` |
| Spec citations of `vulkan_present.rs` | 4, all in `app/06_canvas.md` (472, 473, 599, 600) | same pattern with `vulkan_present\.rs` |
| Spec citations of `canvas/mod.rs` | 47 (44 `app/06_canvas.md`, 2 `tooling/09_debug-report.md`, 1 `tooling/01_project-manifest.md`; see §4.2 table) | `grep -rnoE "\[\[src/codegen/runtime/canvas/mod\.rs:[^]]+\]\]" src/docs/spec \| wc -l` → 47 |
| Spec citations of `runtime_helpers_thread.rs` | 21 in 7 files | `grep -rnoE "\[\[src/codegen/runtime/thread/runtime_helpers_thread\.rs:[^]]+\]\]" src/docs/spec \| wc -l` → 21 |
| Whole-file citations of the four | 0 | `grep -rnoE "\[\[src/codegen/runtime/(canvas/(vulkan\|vulkan_present\|mod)\|thread/runtime_helpers_thread)\.rs\]\]" src/docs/spec \| wc -l` → 0 |
| `.ai` path references | 2: `.ai/arch-abi.md:70`, `.ai/canvas-threading.md:585` | `grep -n "vulkan\.rs\|vulkan_present\|canvas/mod\.rs\|runtime_helpers_thread\|runtime/canvas" .ai/*.md` (the third hit, `:614` `vulkan_handoff.rs`, is unaffected) |
| References naming `vulkan.rs` / `vulkan_present.rs` / `canvas/mod.rs` outside the spec | 27: 19 `vulkan.rs`, 5 `vulkan_present.rs` (excluding the `#[path]` line), 3 `canvas/mod.rs`; 25 are code/shader/MFBASIC comments and 2 are the `.ai` lines above (all listed in §4.6) | `grep -rnE '\bvulkan\.rs\b' src tests tools scripts .ai`, `grep -rnE 'vulkan_present\.rs' …`, `grep -rnE 'canvas/mod\.rs' …`, each minus `src/docs/spec` |
| `#[path]` / `include_*` in V's files | `vulkan.rs:105` `#[path = "vulkan_present.rs"]`; `vulkan.rs:249-250` `include_bytes!("shaders/mfb_canvas.{vert,frag}.spv")`. J owns the test `include_str!` | `grep -n "include_\|#\[path" <four files>` |
| Non-test `#[cfg(...)]` gates in V's files | 0 | `grep -n '#\[cfg(' <four files> \| grep -v 'cfg(test)'` → empty |
| Coverage-exception / guard-allowlist entries | 0 | `grep -rnE 'canvas/vulkan\|vulkan_present\|runtime_helpers_thread\|runtime/canvas' scripts/coverage-exceptions.txt tests/guards/*.rs` → empty |
| `man`, `AGENTS.md`, `.github`, `Cargo.toml`, `scripts`, `tools` references | 0 | `grep -rnE 'runtime/canvas/(vulkan\|vulkan_present\|mod)\.rs\|runtime_helpers_thread' src/docs/man AGENTS.md .github Cargo.toml scripts tools` → empty |
| `#[allow(clippy::too_many_arguments)]` precedents in `vulkan.rs` | 10 | `grep -c 'allow(clippy::too_many_arguments)' src/codegen/runtime/canvas/vulkan.rs` |

The `canvas/mod.rs` citations break down by file as 44 in `app/06_canvas.md`, 2 in
`tooling/09_debug-report.md` and 1 in `tooling/01_project-manifest.md`. One
`GRAPHICS_OFFSET_AUTO_WINDOW_MS` citation is in `tooling/01` and the other is at
`06_canvas.md:2240`.

### Verified properties

- **The citation checker is file-level** (`src/docs/spec/mod.rs::spec_citations_resolve`,
  plan-176-A §2).
  - Converting `vulkan.rs` → `vulkan/mod.rs`, `vulkan_present.rs` → `vulkan/present/` and
    `runtime_helpers_thread.rs` → `runtime_helpers_thread/` makes every old citation fail,
    which forces their sweep.
  - **`canvas/mod.rs` keeps its path.** Its 47 citations would pass silently after their
    symbols move, so their sweep is done by the explicit table in §4.2. That sweep is checked
    by `grep -rnoE '\[\[src/codegen/runtime/canvas/mod\.rs:' src/docs/spec | wc -l` → 0. All
    47 cited symbols move out of `mod.rs`; `GraphicsScratch`, the only thing left, is not
    cited.
- **Glob-cycle imports compile without warnings** (`use super::*;` in a part, `use part::*;` in
  `mod.rs`). Probe `/tmp/loc/probe2` printed `6` with no warning (brief, 2026-10-14).
  - A child's `use super::*;` also imports the parent's private items and private `use`
    imports, including module names such as `present`. That is the same mechanism
    `vulkan_present.rs` and every inline `mod tests { use super::*; }` rely on today.
- **An unused glob import warns.** Probe `/tmp/loc/probe3` printed
  `warning: unused import: pty::*`. So `vulkan/mod.rs` gets a private part glob only for parts
  that hold a `pub(super)` item some other module reaches (§4.3). It gets no glob for `build`
  or `draw_scene`.
- **No `#[cfg]` host gate, so no cross-host `cargo check` is needed.** The Windows-only present
  path is chosen by `has_present_path(platform)` at codegen time, not by `#[cfg]`
  (`grep -n '#\[cfg(' … | grep -v 'cfg(test)'` → empty). It is compiled and exercised on the
  Mac by `-target windows-x86_64`.
- **Moving Rust code cannot change compiler output through source locations** (plan-176-A §2:
  no `module_path!`/`file!`/`line!` in `src`).
- **The checked-in SPIR-V carries no shader source text.** `scripts/regen-spirv.sh` runs
  `glslangValidator -V` without `-g`, and `strings
  src/codegen/runtime/canvas/shaders/mfb_canvas.frag.spv | grep -c vulkan` → 0. The two
  shader comment edits in §4.6 therefore need no `.spv` regeneration.
  - Neither shader edit is inside the generated `@CURVES-BEGIN`/`@CURVES-END` block
    (`mfb_canvas.frag` lines 1161–1447; the edits are at lines 112 and 634). The test
    `the_fragment_shader_holds_the_generated_curve_functions` is unaffected.
- **The canvas runtime reaches exactly one golden.**
  - No `tests/byte-identity/` fixture imports canvas (`.ai/testing-gates.md`; `grep -rl "IMPORT
    canvas" tests/byte-identity` → empty).
  - `tests/syntax/app/app-mouse-surface` builds an `--app` canvas program and carries
    `.ncodesum` goldens for `macos-aarch64`, `linux-x86_64`, `linux-aarch64` and
    `windows-x86_64` (`ls tests/syntax/app/app-mouse-surface/golden`).
  - Both GPU emitters land in it (bug-484 moved all seven of its goldens).
  - `scripts/artifact-gate.sh` only accepts a `tests/byte-identity/<builtin>` selector or `all`.
    The per-phase canvas check is therefore the explicit four-target loop **C1** in the Phases
    preamble, which runs the same build and the same sha256 comparison the gate runs.
- **The thread helpers reach the `thread` and `os` byte-identity fixtures**
  (`ls tests/byte-identity` lists both). `lower_os_sleep_helper` lowers `os::sleep`, and the
  rest lower `thread::`.
- **No guard classifies V's files by name.** `grep -n "canvas\|runtime/\|thread/"
  tests/guards/architecture_guards.rs tests/guards/no_type_strings.rs
  tests/guards/no_operator_strings.rs` gives one hit, `src/target/shared/runtime/usage.rs`, a
  different file. No guard edit is needed. The three guards still run each phase.
- **New names do not collide.** `grep -rnwE
  "emit_record_begin|emit_item_walk|emit_finish_frame|emit_frame_setup|emit_park_arguments|DrawSceneSlots"
  src` → empty. `grep -rnwE "fn (calls|memory|build|…|handle|sleep|queue)"
  src/codegen/runtime/{canvas,thread}` gives one hit, a test-local `fn publish` inside
  `canvas/mod.rs`'s `mod tests`. That is a different module from `vulkan::publish`, so it does
  not collide.
- **`pub(in path)` has precedent** (`grep -rn "pub(in " src | wc -l` → 100, e.g.
  `src/optimizer/opt1/dce.rs:125`).

## 3. Design Overview

**Conventions** (plan-176-A, `.ai/build-tooling.md` "Splitting an over-budget file"):
- `<stem>.rs` → `<stem>/mod.rs` + parts, using `git mv` to `mod.rs` first so history follows.
- Each part starts with `use super::*;` plus the `use` lines the moved text needs that the
  parent does not import. Unused imports are trimmed per file. `mod.rs` keeps the original
  header `use` lines, because children see them through `use super::*`.
- A private item that another part reaches becomes `pub(super)`.
- The surface re-exports from `mod.rs`, so outside paths do not change.
- ≤ 900 code LOC per part.
- Text moves verbatim. rustfmt may re-wrap it, but no token changes.

**Gate class: byte-identity.** Every phase builds `cargo build --release` and then runs:
- the scoped gates for what moved: C1 (the four-target `app-mouse-surface` check) for canvas
  phases, and `sh scripts/artifact-gate.sh target/release/mfb thread` and `… os` for the thread
  phase;
- the moved modules' unit tests and the runtime targets named per phase;
- `cargo check --all-targets` with no new warning;
- `spec_citations_resolve`, the three name-based guards, the §10.1 grep, and the file-size
  tighten.

At letter end, once, V runs the plan-176 final gate in the Validation Plan. A diff is never a
stop:
1. Localize it on ONE target of ONE fixture: rebuild that `.ncode` with a baseline binary from
   the phase's parent commit (`git worktree add --detach /tmp/v-base HEAD~1`, `cargo build
   --release` there), then `diff` the two `.ncode` files.
2. Fix the motion or the extraction.
3. Re-run.

**Order: lowest risk first.**
1. Phase 1, the thread helpers: a separate subsystem with zero visibility changes.
2. Phase 2, the canvas root: three visibility changes, path kept, explicit citation table.
3. Phase 3, `vulkan.rs` → `vulkan/` with `vulkan_present.rs` moved whole to
   `vulkan/present/mod.rs`: 62 visibility changes, plus the `#[path]` removal.
4. Phase 4, `present/` split into parts: 7 visibility changes.
5. Phase 5, the draw-scene extraction, which is the only phase that changes a function boundary.

**Risk concentrates** in two places:
- **Phase 5.** It re-plumbs 58 shared locals of the one linear frame emitter into a struct. It
  is safe because:
  - every allocation and label runs in its original order (the hoist proof in §4.5);
  - the bodies are verbatim apart from `s.` prefixes;
  - the control-flow audit shows the two `return`s and every `?` keep their meaning.
- **Phase 3's name-resolution chain.** `present` → `vulkan` → the parts' private globs, which
  `cargo check` proves.

The emitted code is proven by C1 on all four targets plus `codegen_canvas_vulkan_off_frame_path`,
which runs the emitted Vulkan members in the `-ncode` interpreter. Real GPU behavior is proven
once, by the box scripts in the Validation Plan.

**Intermediate NEW rows** (the guard warns and never fails, plan-176-A §Decisions):
- After Phase 3, `vulkan/draw_scene.rs` (1060) and `vulkan/present/mod.rs` (1848) are over
  1000.
- Phase 4 removes the second row and Phase 5 removes the first.
- Each phase's Acceptance names exactly the rows expected at that point. No ceiling row is
  added by hand for them.

**Rejected alternatives.**
- *Exempt `emit_vulkan_draw_scene`.* V.md proposed it, as one linear emitter. The owner
  decided that single functions over the limit are decomposed by extract-method (plan-176-A
  §Decisions). `/tmp/loc/maps/V-extract.md` shows one struct carries all shared state with no
  logic change.
- *Keep `vulkan_present.rs` a sibling directory, `vulkan_present/`, reached by `#[path =
  "../vulkan_present/mod.rs"]`.* That keeps a load-bearing `#[path]` whose resolution base
  changes with the parent file's name, which is exactly the hazard V.md had to call out. As
  `vulkan/present/` it is an ordinary child module named after its `mod present;`, and the
  tree shows the parent/child relation that `use super::*` already depends on.
- *Return `(usize, usize)` from the park phase instead of hoisting two slots into
  `DrawSceneSlots`.* That is equally byte-identical but needs a second carrier. The hoist is
  proven safe (§4.5), and one struct is simpler.
- *Hoist `off_render_pass` too.* It is allocated after `emit_vulkan_target`, which allocates 10
  slots, so hoisting it would change every one of those offsets. It stays at its statement and
  is returned by `emit_frame_setup`.
- *A private part glob for every vulkan part.* A glob that nothing resolves through warns
  (`/tmp/loc/probe3`). `build` and `draw_scene` have no `pub(super)` items, so they get only
  their explicit `pub(crate) use` re-exports.
- *`pub(crate)` for the five `present` fns `vulkan` calls.* `pub(in
  crate::codegen::runtime::canvas::vulkan)` gives exactly the reach of `PresentSlots`'
  `pub(super)`, which they take as a parameter. So `private_interfaces` cannot fire, and the
  items are no wider than their only caller needs.
- *Leaving stale prose comments.* After V the files they name do not exist, and AGENTS.md says
  never to leave a bug, so every one is repointed (§4.6).

## 4. Detailed Design

LOC figures come from `python3 -I /tmp/loc/span.py <file> <a> <b>` over the stated ranges.
Multi-range values are sums. "guess" marks `mod`/`use` lines and new code not yet written.
Re-measure every new file with `python3 -I tools/loc-census/census.py --file <path>` before
committing.

### 4.1 `thread/runtime_helpers_thread.rs` → `thread/runtime_helpers_thread/` (Phase 1)

Ranges at a3c95fc07:

| New file | Lines | Items | Code LOC |
|---|---|---|---|
| `runtime_helpers_thread/mod.rs` | 1–293 | `use` lines, `ThreadBodyParts`, `ThreadVregs` + impl, `ThreadSimpleOp`, `emit_thread_deadline`, `worker_arena_state_size`, `emit_block_lock`, `emit_block_unlock`, and the private queue primitives `emit_queue_cond`, `emit_queue_wait`, `emit_close_queue_locked`, `emit_wake_queue_locked`, `emit_close_resource_queues_locked`, `emit_release_reference`; plus `mod handle; mod sleep; mod queue;` and `pub(crate) use handle::*; pub(crate) use sleep::*; pub(crate) use queue::*;` | 231 + 6 (guess) |
| `runtime_helpers_thread/handle.rs` | 294–616 and 1351–1664, in source order | `simple_thread_handle_helper`, `thread_is_cancelled_helper`, `lower_thread_finish` | 286 + 242 = 528 |
| `runtime_helpers_thread/sleep.rs` | 617–828 | `CancellableSleepSlots`, `emit_cancellable_sleep_wait`, `emit_cancellable_sleep_interrupted`, `lower_os_sleep_helper` | 149 |
| `runtime_helpers_thread/queue.rs` | 829–1350 | `thread_queue_write_helper`, `ThreadReadMode`, `thread_queue_read_helper` | 454 |

The directory stem differs from the sibling `runtime_helpers.rs`, and no part is named
`runtime_helpers`.

**Surface.** `thread/mod.rs:7-8` (`pub(crate) mod runtime_helpers_thread; pub(crate) use
runtime_helpers_thread::*;`) is unchanged.

**Visibility.** 0 changes. All six cross-part private items are in `mod.rs`, which every child
sees. The siblings `runtime_helpers.rs`, `control_block.rs` and `value_functions.rs` use none of
them (V.md xref).

**`lower_thread_finish`'s new `program_shutdown` block** (e9620387b) uses
`crate::codegen::error::report::ERROR_REPORT_SYMBOL` by full path and `internal_branch`. It
needs no new `use` line in `handle.rs` beyond what `use super::*` gives.

**Citations (21).** Each `[[src/codegen/runtime/thread/runtime_helpers_thread.rs:sym]]` becomes:

| Symbol | New file | Count | Where |
|---|---|---|---|
| `emit_block_lock` | `runtime_helpers_thread/mod.rs` | 1 | threading/07:100 |
| `worker_arena_state_size` | `runtime_helpers_thread/mod.rs` | 1 | threading/06:107 |
| `simple_thread_handle_helper` | `runtime_helpers_thread/handle.rs` | 5 | threading/11:24, 07:173, 08:126, 08:137, 08:168 |
| `thread_is_cancelled_helper` | `runtime_helpers_thread/handle.rs` | 1 | threading/07:85 |
| `lower_thread_finish` | `runtime_helpers_thread/handle.rs` | 5 | language/08:286, threading/09:66, 08:202, 06:184, 06:210 |
| `lower_os_sleep_helper` | `runtime_helpers_thread/sleep.rs` | 1 | threading/06:143 |
| `thread_queue_write_helper` | `runtime_helpers_thread/queue.rs` | 4 | language/15:349, threading/08:50, 08:89, 08:124 |
| `thread_queue_read_helper` | `runtime_helpers_thread/queue.rs` | 3 | threading/08:51, 08:88, 08:125 |

Unaffected:
- `lower_thread_start_helper` is in `runtime_helpers.rs`, not in this file. Its citations
  and the `.ai/canvas-threading.md` §2 mention stay as they are.
- The `src/codegen/engine/builder/` doc comment naming
  `runtime_helpers_thread::thread_is_cancelled_helper` is a module path, and it still resolves
  through the re-export.

### 4.2 `canvas/mod.rs` → `canvas/mod.rs` + six siblings (Phase 2)

| File | Lines (as of 17a9b5799) | Items | Code LOC |
|---|---|---|---|
| `canvas/mod.rs` | 1–77, 4056–4284 | `//!` header, the existing `mod` decls (34–38) and `use` lines (39–47), `GraphicsScratch` + impl (stays so its private fields remain visible to every child); the six new `mod` decls and `pub(crate) use <part>::*;` re-exports; the inline `#[cfg(test)] mod tests` (178 test LOC) | 32 + 12 (guess) |
| `canvas/state_layout.rs` | 78–509 | `GRAPHICS_STATE_SYMBOL`, every `GRAPHICS_OFFSET_*` (incl. MTL, VULKAN, WSI, POINTER), `BLEND_MODE_COUNT`, `FRAME_RING_SLOTS`, `FRAME_COUNTER_*`, `VULKAN_MAX_SWAPCHAIN_IMAGES`, `GRAPHICS_POINTER_WORDS`, `GRAPHICS_STATE_SIZE`, `WAKE_*` | 124 |
| `canvas/scene_layout.rs` | 510–1703 | item block (`ITEM_*`, `ITEM_RESIDUAL_FIELDS`, `PI_F64_BITS`, `emit_item_residual_field`, `emit_item_stroke_sign`, `emit_item_arc_flags`, `SQRT_2_F64_BITS`, `emit_square_cap_reach`, `emit_item_polygon_rule`), `HEADER_*`, `GEO_KIND_*` (incl. the cfg(test) `GEO_KIND_GROUP` at 1066–1067), glyph and tile constants, GPU buffer sizing (`VULKAN_*`/`METAL_*` buffer constants, `BAND_*`, `CANVAS_DRAW_*`, both `const _: () = assert!`, `VULKAN_SPEC_CONSTANTS`) | 402 (+2 test: `GEO_KIND_GROUP`) |
| `canvas/entry.rs` | 1704–2114 | `graphics_trampoline_shadow`, `thread_entry_frame`, `graphics_trampoline_saved`, `GRAPHICS_TRAMPOLINE_SYMBOL`, `graphics_state_data_object`, `CANVAS_PROFILE_*`, `profile_data_object`, `emit_profile_word`, `render_loop_symbol`, `emit_graphics_trampoline`, `emit_thread_entry` | 194 |
| `canvas/thread_lifecycle.rs` | 2115–2842 | `state_base`, `emit_signal_redraw`, `emit_wait_for_redraw`, `emit_stop_graphics`, `emit_start_graphics`, `emit_graphics_spawn` | 568 |
| `canvas/frame_sync.rs` | 2843–3623 | `FrameRecord`, `emit_ring_push`, `emit_bump`, `emit_note_present`, `emit_graphics_lock`, `emit_frame_counters`, `emit_frame_done`, `emit_frame_cond_broadcast`, `CLOCK_REALTIME`, `NANOS_PER_SECOND`, `emit_realtime_nanos`, `emit_await_pickup`, `emit_sync_frame`, `emit_set_sync_mode` | 668 |
| `canvas/surface.rs` | 3624–4055 | `DEFAULT_SURFACE_*`, `SurfaceDimension` + impl, `emit_surface_dimension`, `emit_skip_unless_autopaint_live`, `PointerRegs`, `emit_publish_pointer`, `emit_publish_surface_size{,_tagged}`, `emit_publish_display_rate`, `emit_display_milli_hz`, `emit_set_display_override`, `SCRATCH_RESIZE`, `emit_did_resize`, `emit_set_gpu_mode`, `emit_use_gpu`; the cfg(test) `signal_redraw_arguments` (3725–3750, 21 test LOC) | 273 (+21 test) |

None of the new names collides with the existing `curve_shader`, `metal`, `vulkan`,
`vulkan_handoff` or `shaders/`.

**Surface.**
- `mod.rs` declares `mod state_layout; mod scene_layout; mod entry; mod thread_lifecycle; mod
  frame_sync; mod surface;` and re-exports each with `pub(crate) use <part>::*;`.
- Every caller keeps using `crate::codegen::runtime::canvas::X`. That includes
  `process_lifecycle.rs`'s `canvas::GraphicsScratch`, the `use crate::codegen::runtime::canvas::{…}`
  lines in `vulkan.rs`, `vulkan_present.rs`, `metal.rs` and `src/target/**`, and the
  `signal_redraw_arguments` users in `src/target/{linux_gtk/pointer.rs,
  macos_aarch64/app/mouse_view.rs, win_x86_64/app/{pointer.rs,mod.rs}}`.
- The inline `mod tests` uses only re-exported names: `emit_publish_surface_size`,
  `DEFAULT_SURFACE_*`, `GRAPHICS_OFFSET_{WIDTH,HEIGHT,RESIZES}`, `graphics_trampoline_saved`,
  `graphics_trampoline_shadow` and `thread_entry_frame`. All are `pub(crate)`
  (`grep -nE '^pub\(crate\) (const|fn) (…)' src/codegen/runtime/canvas/mod.rs`).
- Each part starts with `use super::*;` plus the subset of lines 39–47 it needs. `mod.rs`'s own
  `use` lines are then trimmed to what the header, `GraphicsScratch` and the children need.

**Visibility.** 3 changes, each private → `pub(super)`: `emit_frame_cond_broadcast`,
`NANOS_PER_SECOND` and `emit_realtime_nanos`. All three are defined in `frame_sync` and used in
`thread_lifecycle`.
- `SurfaceDimension::offset` and `::startup` are called only inside `surface` (3670, 3676,
  3690) and stay private.
- No sibling uses a private item of `mod.rs` (V.md grep).

**Citations (47).** Each `[[src/codegen/runtime/canvas/mod.rs:sym]]` becomes
`[[src/codegen/runtime/canvas/<part>.rs:sym]]`. Destination parts were verified by locating each
symbol's definition line in the ranges above (45 distinct symbols: entry 2, frame_sync 4,
scene_layout 30, state_layout 2, surface 5, thread_lifecycle 2).

| Part | Symbols (spec file:line) |
|---|---|
| `state_layout.rs` (3) | `GRAPHICS_OFFSET_AUTO_WINDOW_MS` (tooling/01:136, app/06:2240), `GRAPHICS_OFFSET_POINTER_SEQ` (06:2107) |
| `scene_layout.rs` (30) | `METAL_MAX_FRAME_ITEMS` (06:423), `VULKAN_MAX_FRAME_ITEMS` (424), `VULKAN_SPEC_CONSTANTS` (444), `CANVAS_DRAW_PASS_WORD` (467), `HEADER_POLYLINE_FIRST` (487), `EDGE_RESIDUAL_SCALE` (546), `METAL_EDGE_RESIDUAL_BASE_WORDS` (547), `VULKAN_EDGE_RESIDUAL_BASE_WORDS` (548), `ITEM_OFFSET_SHAPE_LO` (549), `ITEM_RESIDUAL_FIELDS` (550), `ITEM_ARC_REFLEX` (551), `emit_item_polygon_rule` (552), `emit_item_stroke_sign` (553), `emit_square_cap_reach` (697), `HEADER_JOIN_COUNT` (736), `HEADER_JOIN_REACH` (737), `HEADER_DASH_COUNT` (796), `ITEM_OFFSET_STROKE_STYLE` (804), `HEADER_POLYGON_RULE` (836), `HEADER_CLIP_MODE` (1036), `ITEM_OFFSET_CLIP_BAND` (1053), `ITEM_OFFSET_CLIP_PATH` (1054), `HEADER_PICTURE_TRANSPARENCY` (1068), `HEADER_GRADIENT_SPREAD` (1136), `ITEM_SURFACE_GRADIENT_KIND` (1143), `ITEM_OFFSET_GRADIENT_INV` (1144), `ITEM_OFFSET_GRADIENT_FOCAL` (1145), `CURVE_PIECE_SLOTS` (1272), `CANVAS_DRAW_ENTRY_WORDS` (1559), `ITEM_RIM_WORDS` (1582) |
| `entry.rs` (2) | `CANVAS_PROFILE_SYMBOL` (tooling/09:232), `emit_profile_word` (tooling/09:294) |
| `thread_lifecycle.rs` (3) | `emit_wait_for_redraw` (06:1236, 06:2337), `emit_signal_redraw` (06:2335) |
| `frame_sync.rs` (4) | `emit_note_present` (06:1183), `emit_frame_done` (1184), `emit_frame_counters` (1185), `emit_await_pickup` (1235) |
| `surface.rs` (5) | `emit_publish_surface_size_tagged` (06:2076), `SurfaceDimension` (2080), `emit_did_resize` (2086), `emit_publish_pointer` (2117), `emit_skip_unless_autopaint_live` (2325) |

`app/06` line numbers are as of a3c95fc07 without the uncommitted bug-1092 hunk. That hunk adds
10 lines after line 2587, so citations past it shift by 10 once it lands. Symbol names are
authoritative.

**§10.1 holds.** Every `GRAPHICS_OFFSET_VULKAN_*` stays in `state_layout.rs` under
`src/codegen/runtime/canvas/`, and `grep -rn GRAPHICS_OFFSET_VULKAN src/target` stays empty
(run at HEAD 2026-10-17: empty).

### 4.3 `canvas/vulkan.rs` → `canvas/vulkan/` (Phase 3)

The file becomes `vulkan/mod.rs` plus 14 parts, split along the builder-thread, graphics-thread
and shared-plumbing lines. All ranges are as of 17a9b5799 and unchanged after J. The parts add
up to the whole file:
495+323+254+576+503+585+540+439+546+343+427+344+752+1060+113 = 7300.

| New file | Lines | Thread | Items | Code LOC |
|---|---|---|---|---|
| `vulkan/mod.rs` | 1–407, 458–693, 2581–2691, 3643–3694, 9012–9046, and J's `#[cfg(test)] mod tests;` | shared | `//!` doc and every `use` line (1–104); `mod present;` (replaces 105–106, §Hazards); `pub(crate) use present::{CANVAS_CHILD_SYMBOL, WM_APP_CANVAS_SHOW};` (107); loader names, probe entry points, `ST_*` and enum constants, `SPIRV_VERTEX`/`SPIRV_FRAGMENT`, the `*_symbol` fns, `data_objects`, `has_vulkan_backend`, every C-struct layout constant (458–693, 2581–2691, 3643–3694, incl. `PUSH_OFFSET_BYTES` and `FLOAT_ONE_BITS_F32`), cfg(test) `SPIRV_MAGIC` (255–256) and `assert_layout` (9012–9046); the 14 `mod` decls, the private globs and the re-exports below | 495 measured + 30 decl/use lines = 525 (guess) |
| `vulkan/calls.rs` | 408–457, 3274–3357, 8666–9011 | shared | `Field`, `emit_struct`, `emit_create_call`, `emit_device_call_1`, `state_base_into`, `emit_state_store`, `emit_state_load`, `emit_store_f32_from_integer`, `emit_int_arg_zero`, `emit_int_arg`, `emit_int_arg_slot`, `emit_addr_arg`, `emit_call_fn`, `emit_open_library`, `emit_dlsym`, `emit_zero_range` | 323 |
| `vulkan/memory.rs` | 3358–3642 | shared (descriptors + target) | `emit_allocate_and_bind`, `emit_pick_memory_type`, `emit_memtype_scan` | 254 |
| `vulkan/build.rs` | 694–1357 | builder entry + ready-word hand-off | `emit_vulkan_build`, `emit_publish_state`, `emit_spawn_builder`, `emit_vulkan_ready` | 576 |
| `vulkan/descriptors.rs` | 1358–1920 | builder | `emit_vulkan_descriptors` | 503 |
| `vulkan/pipeline.rs` | 1921–2580 | builder | `emit_vulkan_pipeline` | 585 |
| `vulkan/target.rs` | 2692–3273 | graphics (render target) | `emit_vulkan_target` | 540 |
| `vulkan/item_block.rs` | 3695–4243 | graphics | `emit_item_block`, `emit_gradient_upload` | 439 |
| `vulkan/edges.rs` | 4244–4859 | graphics | `emit_edge_upload`, `emit_clip_upload` | 546 |
| `vulkan/stroke_style.rs` | 4860–5234 | graphics | `emit_dash_upload`, `emit_join_upload` | 343 |
| `vulkan/bands.rs` | 5235–5794 | graphics | `BandSlots` + impl, `BandSet`, `ITEM_BANDS`, `CLIP_BANDS`, `emit_band_index`, `emit_band_range` | 427 |
| `vulkan/picture.rs` | 5795–6247 | graphics | `emit_picture_upload`, `PictureSlots`, `PICTURE_HASH_MULTIPLIER`, `emit_picture_lookup`, `emit_picture_record` | 344 |
| `vulkan/publish.rs` | 6248–7307 | graphics | `emit_item_publish`, `emit_split_or_publish`, `emit_draw_list_pass`, `GlyphPublishSlots`, `emit_glyph_publish` | 752 |
| `vulkan/draw_scene.rs` | 7308–8542 | graphics | `emit_vulkan_draw_scene` only; becomes `vulkan/draw_scene/` in Phase 5 (§4.5) | 1060 |
| `vulkan/passes.rs` | 8543–8665 | graphics (multi-pass frame control) | `emit_branch_on_pass`, `emit_load_render_pass` | 113 |

The test module is plan-176-J's single test-only file `vulkan/tests/mod.rs` (748 LOC at planning,
under the 1000 test-only limit, so J does not split it). The `#[cfg(test)] mod tests;` line J left
at the end of `vulkan.rs` moves into `vulkan/mod.rs`, and `vulkan/tests/mod.rs` is already in
place. J's `include_str!("../../shaders/mfb_canvas.frag")` in `vulkan/tests/mod.rs` resolves the
same afterwards: `vulkan/tests/` does not move. The name `tests` does not collide: there is no
other `tests` item in `vulkan`, and `present`'s inline `tests` is a different module.

**Surface and visibility.**
- `vulkan/mod.rs` re-exports `pub(crate) use build::{emit_vulkan_build, emit_vulkan_ready};` and
  `pub(crate) use draw_scene::emit_vulkan_draw_scene;`. It keeps `pub(crate) use
  present::{CANVAS_CHILD_SYMBOL, WM_APP_CANVAS_SHOW};`.
- `data_objects`, `has_vulkan_backend` and the other `pub(crate)` loader/symbol fns stay in
  `mod.rs`.
- External callers are unchanged:
  - `func_graphics.rs` (`emit_vulkan_build`, `emit_vulkan_ready`)
  - `func_metal_draw.rs` (`emit_vulkan_draw_scene`)
  - `engine/builder/mod.rs` (`data_objects`, `has_vulkan_backend`)
  - `src/target/win_x86_64/app/mod.rs` (`CANVAS_CHILD_SYMBOL`, `WM_APP_CANVAS_SHOW`)
- Private part globs in `mod.rs`: `use calls::*; use memory::*; use descriptors::*; use
  pipeline::*; use target::*; use item_block::*; use edges::*; use stroke_style::*; use
  bands::*; use picture::*; use publish::*; use passes::*;`. That is the 12 parts with a
  `pub(super)` item. `build` and `draw_scene` have none and get no glob (§3 rejected
  alternatives).
  - Siblings, `present` and `tests` resolve the `pub(super)` names through `use super::*`.
  - Parts reach `present::X` through the module name `present`, which `use super::*` brings
    in. The call sites are in `build.rs`, at original lines 834–836 and 1092–1094, and in
    `draw_scene.rs`, at 7325–8330. `data_objects` calls it at 394–395 in `mod.rs`.
- **62 `pub(super)` additions: 39 items and 23 fields.** `pub(super)` in `vulkan/<part>.rs`
  means visible in `vulkan` and all its descendants.
  - `calls.rs` (16): `Field`, `emit_struct`, `emit_create_call`, `emit_device_call_1`,
    `state_base_into`, `emit_state_store`, `emit_state_load`, `emit_store_f32_from_integer`,
    `emit_int_arg_zero`, `emit_int_arg`, `emit_int_arg_slot`, `emit_addr_arg`, `emit_call_fn`,
    `emit_open_library`, `emit_dlsym`, `emit_zero_range`.
  - `descriptors.rs`: `emit_vulkan_descriptors`. `pipeline.rs`: `emit_vulkan_pipeline`.
    `target.rs`: `emit_vulkan_target`. `memory.rs`: `emit_allocate_and_bind`.
  - `item_block.rs`: `emit_item_block`, `emit_gradient_upload`. `stroke_style.rs`:
    `emit_dash_upload`, `emit_join_upload`. `edges.rs`: `emit_edge_upload`, `emit_clip_upload`.
  - `bands.rs`: `BandSlots`, `BandSlots::allocate`, `BandSet` (so the pub consts are not a
    private interface), `ITEM_BANDS`, `CLIP_BANDS`, `emit_band_index`, plus the field
    `BandSlots.cursor`.
  - `picture.rs`: `emit_picture_upload`, `PictureSlots` and all 6 of its fields. It is built
    by a struct literal in `draw_scene`.
  - `publish.rs`: `emit_split_or_publish`, `emit_draw_list_pass`, `GlyphPublishSlots` and all
    16 of its fields, `emit_glyph_publish`.
  - `passes.rs`: `emit_load_render_pass`. `emit_branch_on_pass` is already `pub(super)`.
  - Source of the crossings: V.md `xref.py src/codegen/runtime/canvas/vulkan.rs
    "mod:1-407,458-693,2581-2691,3643-3694,9012-9046" "build:694-1357" … "tests:9047-9862"`,
    plus the field greps (`sed -n 7308,8542p … | grep -o 'bands\.[a-z_]*'`).
- What `present` reaches through `use super::*`:
  - consts, all of which stay in `mod.rs`;
  - `emit_branch_on_pass`;
  - from `calls.rs`: `Field`, `emit_struct`, `emit_addr_arg`, `emit_call_fn`,
    `emit_create_call`, `emit_device_call_1`, `emit_dlsym`, `emit_int_arg`,
    `emit_int_arg_slot`, `emit_state_load`, `emit_state_store`, `emit_zero_range`;
  - in its tests, `assert_layout` (in `mod.rs`).

**Hazards.**
- **`#[path]` removed; `vulkan_present.rs` moves whole.** `git mv
  src/codegen/runtime/canvas/vulkan_present.rs src/codegen/runtime/canvas/vulkan/present/mod.rs`.
  Lines 105–106 (`#[path = "vulkan_present.rs"]` / `mod present;`) become the single line `mod
  present;`, which resolves to `vulkan/present/mod.rs`.
  - Its first line, `use super::*;`, still names `vulkan`, and its `pub(super)` items still
    mean "visible in `vulkan`". So this phase changes no visibility in it.
  - It declares no file submodules (`grep -n "^mod \|#\[path"` → only the inline `mod tests` at
    2064).
- **`include_bytes!`**: lines 249–250, `include_bytes!("shaders/mfb_canvas.{vert,frag}.spv")`,
  become `include_bytes!("../shaders/mfb_canvas.vert.spv")` and `"../shaders/mfb_canvas.frag.spv"`
  in `vulkan/mod.rs`.
- **Spec citations (14, all `app/06_canvas.md`).** Each `[[src/codegen/runtime/canvas/vulkan.rs:sym]]`
  becomes `[[src/codegen/runtime/canvas/vulkan/<file>:sym]]`:
  - `has_vulkan_backend` (335) → `mod.rs`; `PUSH_OFFSET_BYTES` (1569) → `mod.rs`
  - `emit_vulkan_build` (352) → `build.rs`
  - `emit_vulkan_draw_scene` (397, 598) → `draw_scene.rs`. Phase 5 repoints them again (§4.5).
  - `emit_band_index` (446) → `bands.rs`
  - `emit_load_render_pass` (471) → `passes.rs`
  - `emit_edge_upload` (485, 556, 1404) → `edges.rs`; `emit_clip_upload` (1056) → `edges.rs`
  - `emit_dash_upload` (806) → `stroke_style.rs`
  - `emit_picture_lookup` (993) → `picture.rs`
  - `emit_draw_list_pass` (1568) → `publish.rs`
- **Spec citations of `vulkan_present.rs` (4).** All four become
  `[[src/codegen/runtime/canvas/vulkan/present/mod.rs:sym]]` in this phase: `emit_present_resume`
  (472), `emit_submit_and_present` (473, 600) and `emit_ensure_swapchain` (599). Phase 4
  repoints three of them to their parts.
- **`.ai/arch-abi.md:70`**: `runtime/canvas/vulkan.rs:emit_int_arg_zero` →
  `runtime/canvas/vulkan/calls.rs:emit_int_arg_zero`.
- **`.ai/canvas-threading.md:585`**: ``(bug-688 V6, `runtime/canvas/vulkan_present.rs`,
  `canvas::vulkanPresentScene`)`` → ``(bug-688 V6, `runtime/canvas/vulkan/present/`,
  `canvas::vulkanPresentScene`)``. Phase 4 adds ``; record-and-present in
  `vulkan/present/submit.rs` ``. `canvas::vulkanPresentScene` is the builtin
  (`lower_vulkan_present_scene` in `src/codegen/builtins/canvas/func_metal_draw.rs`) and does
  not change.
- **Threading constraint** (§10, §10.1): everything stays under
  `src/codegen/runtime/canvas/vulkan/`. `emit_vulkan_build` moves verbatim into `build.rs`, so
  it still allocates nothing from the arena. `emit_thread_entry` moved to `canvas/entry.rs` in
  Phase 2 and is reached as `canvas::emit_thread_entry` through the re-export.
- **None present**: no respawn, `--exact`, `module_path!` or source-read-by-path in the file
  (`grep -n "read_to_string\|CARGO_MANIFEST\|module_path"` → empty).

### 4.4 `vulkan/present/mod.rs` → `vulkan/present/` parts (Phase 4)

| File | Lines (of the original `vulkan_present.rs`) | Items | Code LOC |
|---|---|---|---|
| `present/mod.rs` | 1–181, 379–520, 2063–2233 | header, `use` lines, every struct-offset and enum constant (private; children see them via `use super::*`), `CANVAS_HWND_SYMBOL`, `CANVAS_CHILD_SYMBOL`, `WM_APP_CANVAS_SHOW`, `WSI_*`, `extension_name_symbol`, `wsi_data_objects`, `IMAGE_LAYOUT_PRESENT_SRC`, `has_present_path`; `PresentSlots` + impl (stays here: its private fields `acquired`, `swapchain`, `sem_*`, `caps`, … are used by swapchain, teardown and submit), `emit_present_begin`, `emit_present_resume`, `emit_require_acquired`, `emit_park_framebuffer`, `park_state`; the inline tests (164 test LOC; they use only `mod.rs` items and `vulkan`'s `assert_layout`) | 219 + 12 (guess) |
| `present/wsi.rs` | 182–378 | `emit_name_array`, `emit_wsi_create_instance`, `emit_create_instance_call`, `emit_wsi_create_device`, `emit_create_device_call` | 182 |
| `present/swapchain.rs` | 521–1418 | `emit_ensure_swapchain` (one fn, 844; no headroom for anything else) | 844 |
| `present/render_pass.rs` | 1419–1539 | `emit_present_render_pass` | 117 |
| `present/teardown.rs` | 1540–1762 | `emit_teardown`, `emit_present_abandon`, `emit_destroy` | 210 |
| `present/submit.rs` | 1763–2062 | `emit_submit_and_present` | 276 |

**Surface.** `present/mod.rs` declares `mod wsi; mod swapchain; mod render_pass; mod teardown; mod
submit;` and adds:

```rust
pub(super) use self::{
    submit::emit_submit_and_present,
    swapchain::emit_ensure_swapchain,
    teardown::emit_present_abandon,
    wsi::{emit_wsi_create_device, emit_wsi_create_instance},
};
use render_pass::emit_present_render_pass;
use teardown::emit_teardown;
```

`vulkan`'s `present::X` call sites and the `pub(crate) use present::{CANVAS_CHILD_SYMBOL,
WM_APP_CANVAS_SHOW};` re-export are unchanged. The two private imports let `swapchain.rs` reach
`emit_present_render_pass` and `emit_teardown` through `use super::*`, so its body stays verbatim.
Each part starts with `use super::*;`.

**Visibility (7).**
- 5 change from `pub(super)` to `pub(in crate::codegen::runtime::canvas::vulkan)`:
  `emit_wsi_create_instance`, `emit_wsi_create_device`, `emit_ensure_swapchain`,
  `emit_present_abandon`, `emit_submit_and_present`.
  - These items now sit one level deeper and `vulkan` must still reach them.
  - The re-export cannot exceed the item's own visibility (E0364).
  - The reach is exactly that of `PresentSlots`' `pub(super)`, so `private_interfaces` does
    not fire.
- 2 change from private to `pub(super)`: `emit_present_render_pass` and `emit_teardown`. Both
  are used by `swapchain` (V.md `xref.py … "render_pass:1419-1539" "teardown:1540-1762"`).

**Citations.**
- `emit_submit_and_present` (472/473, 600) → `vulkan/present/submit.rs`.
- `emit_ensure_swapchain` (599) → `vulkan/present/swapchain.rs`.
- `emit_present_resume` (472) stays `vulkan/present/mod.rs`.

### 4.5 Extract-method: `emit_vulkan_draw_scene` (Phase 5)

Function: `pub(crate) fn emit_vulkan_draw_scene`, original `vulkan.rs:7308-8541`, in
`vulkan/draw_scene.rs` after Phase 3. It is 1060 code LOC (`span.py` 7308–8542). Phase 5 makes
`vulkan/draw_scene.rs` the directory module `vulkan/draw_scene/` (`git mv` to
`draw_scene/mod.rs`). `vulkan/mod.rs` keeps `mod draw_scene;` and `pub(crate) use
draw_scene::emit_vulkan_draw_scene;`.

**Regions** (`bash /tmp/loc/spans.sh <file> a-b …`; original line numbers):

| id | lines | LOC | content |
|---|---|---|---|
| R0 | 7308–7323 | 13 | signature (10 params) |
| R1 | 7324–7330 | 7 | `present_mode`; the guard (`has_vulkan_backend` / `present::has_present_path`), with an early `return Ok(())` |
| R2a | 7331–7338 | 8 | 8 frame labels: `done`, `unavailable`, `item_head`, `item_done`, `item_next`, `text_item`, `swizzle_head`, `swizzle_done` |
| R2b | 7340–7405 | 48 | 47 `allocate_stack_object` slots, `BandSlots::allocate` (7391, 8 more slots), `swapchain = present_mode.then(\|\| PresentSlots::allocate(builder))` (7405) |
| R3 | 7407–7541 | 106 | parks the arguments; allocates `off_draw_entries` (7459) and `off_pass` (7483) mid-region, and creates the label `vk_no_pass_word` (7485) |
| R4 | 7543–7661 | 105 | `emit_present_begin`; `state_base_into`; resume (label `vk_sc_not_resumed`); ready check; `emit_vulkan_target`; allocates `off_render_pass` (7597); first/later pass choice (labels `vk_first_pass`, `vk_pass_chosen`; `emit_load_render_pass`, `emit_ensure_swapchain`) |
| R5 | 7663–7939 | 252 | record: begin the command buffer, clear value, `vkCmdBeginRenderPass`, bind pipeline and descriptor sets, viewport and scissor, resolve `vkCmdDraw` and `vkCmdBindPipeline` once |
| R6 | 7941–8280 | 285 | initialise the cursors; the item loop `item_head` → `item_done`, incl. the blend-mode block (label `vk_same_mode`, 8061) and the shape/text fork |
| R7 | 8282–8541 | 236 | `emit_draw_list_pass`, `vkCmdEndRenderPass`, the present tail (`return Ok(())` at 8345), copy out (label `vk_end_recording`, 8352), end, submit, wait, BGRA swizzle, the `unavailable`/`done` labels, `Ok(())` |

Sum: 13+7+8+48+106+105+252+285+236 = 1060.

**Control-flow audit** (`sed -n 7308,8541p … | grep -n "return\|?;\|)?\|break\|continue\|'[a-z_]*:\||"`):
- There are exactly two `return`s.
  - 7329 (R1) returns `Ok(())` from the guard. R1 stays inline, so it is unchanged.
  - 8345 (R7) returns `Ok(())` at the end of the `if let Some(at) = &swapchain` present tail.
    R7 becomes `emit_finish_frame`, whose call is the tail expression of
    `emit_vulkan_draw_scene`. Its inner `return Ok(())` therefore returns `Ok(())` to the
    caller, which returns it unchanged, and nothing in the original followed R7. The semantics
    are identical.
- Every `?` is on `Result<_, String>`. R4 has them at 7592, 7630 and 7652. R5 has them at 7692,
  7773, 7799, 7832, 7900, 7927 and 7939; the one at 7900 is inside a `for` over two
  `(name, slot)` pairs and exits the fn, not the loop. R7 has them at 8297, 8307, 8327, 8338,
  8389, 8425, 8452 and 8476.
  - Each extracted fn returns `Result<_, String>`, and its call site applies `?` (or is R7's
    tail expression).
  - So emission stops at the same instruction, and the same `Err` string propagates.
- R3 and R6 contain no `?` and no `return`, so they become plain `fn … ()` calls.
- No `break`, no `continue`, no labelled block, no immediately-invoked closure.
- There is one closure, at 7405 (`present_mode.then(|| present::PresentSlots::allocate(builder))`).
  It is evaluated eagerly where it is written, and it moves into `DrawSceneSlots::allocate`
  unchanged.
- The inner `{ … }` blocks at 7484–7499, 7611–7661 and 8060–8159 are scoping only. Each moves
  whole with its region.

**Shared state: one private struct.** The slots `off_state`, `off_handle`, `off_fn`, `off_width`
and `off_height` are used in R3 through R7, and almost every slot crosses a phase boundary.
- So one struct, `DrawSceneSlots`, carries them all. It is built once, after the guard.
- Every phase takes it as `&DrawSceneSlots` and never mutates it. Each slot is a `usize` frame
  offset and each label a `String`; what the emitted code mutates is the frame memory they name.

```rust
/// Every frame label and stack slot `emit_vulkan_draw_scene` shares across its phases.
/// Built once by `allocate`, in the original allocation order (byte-identity).
struct DrawSceneSlots {
    // labels (created first, 7331-7338)
    done: String, unavailable: String, item_head: String, item_done: String,
    item_next: String, text_item: String, swizzle_head: String, swizzle_done: String,
    // slots (7340-7404), field name = the old local name, `off_` prefix kept
    off_state: usize, off_handle: usize, off_fn: usize, off_out: usize, off_surface: usize,
    off_width: usize, off_height: usize, off_geometry: usize, off_offsets: usize,
    off_draws: usize, off_count: usize, off_index: usize, off_item: usize, off_begin: usize,
    off_clear: usize, off_pass_begin: usize, off_viewport: usize, off_scissor: usize,
    off_copy: usize, off_submit: usize, off_cmd_handle: usize, off_desc_set_handle: usize,
    off_edge_cursor: usize, off_grad_cursor: usize, off_dash_cursor: usize,
    off_item_cursor: usize, off_run_start: usize, off_run_count: usize,
    off_bound_mode: usize, off_item_mode: usize, off_pipe_fn: usize,
    off_saved_stroke: usize, off_header: usize, off_glyph_meta: usize, off_glyph_cov: usize,
    off_glyph_cursor: usize, off_pic_count: usize, off_pic_slot: usize,
    bands: BandSlots,
    off_glyph_index: usize, off_glyph_count: usize, off_glyph_w: usize, off_glyph_h: usize,
    off_glyph_x: usize, off_glyph_y: usize, off_glyph_instance: usize, off_draw_fn: usize,
    swapchain: Option<present::PresentSlots>,
    // hoisted out of R3 (hoisting proof below)
    off_draw_entries: usize,
    off_pass: usize,
}
```

The struct has 58 fields: 8 labels, 47 plain slots, `bands`, `swapchain` and the 2 hoisted slots.
It needs no derives. `BandSlots` and `PresentSlots` are not `Clone`, and the struct holds them by
value.

**`fn allocate(builder: &mut CodeBuilder, present_mode: bool) -> DrawSceneSlots`**:
1. R2a, then R2b, verbatim as sequential `let` statements.
2. Then `let off_draw_entries = builder.allocate_stack_object("vk_draw_entries", 8);` and `let
   off_pass = builder.allocate_stack_object("vk_pass_word", 8);`, in that order.
3. Last, `DrawSceneSlots { done, unavailable, …, off_pass }` in field shorthand.

Building the literal from `let` bindings pins evaluation order to statement order, so the field
order of the literal is irrelevant.

**Hoisting proof** for `off_draw_entries` (orig 7459) and `off_pass` (orig 7483):
- `allocate_stack_object` (`src/codegen/engine/regalloc/builder_registers.rs:212`) touches only
  `stack_size` and `stack_slots`, and its slot name embeds `stack_slots.len()`.
- `label` (`:235`) touches only `next_label`.
- `emit` (`:242`) only pushes to `instructions`.
- Between 7405 (the last R2b allocation) and 7459, and between 7459 and 7483, the code makes
  only `builder.emit(...)` calls: no helper call, no label, no allocation (7407–7483 read in
  full).
- So the `allocate_stack_object` sequence is unchanged: `… vk_draw_fn, <PresentSlots ×17 when
  present_mode>, vk_draw_entries, vk_pass_word`. The `next_label` sequence is unchanged too:
  `vk_no_pass_word` is still the first label after the 8 frame labels.
- `vulkan.rs` is unchanged from 17a9b5799 to a3c95fc07 (`git diff --stat 17a9b5799 HEAD --
  src/codegen/runtime` lists only `runtime_helpers_thread.rs`), and V's prerequisite keeps it
  clean, so the read holds at implementation. Phase 5's first task records the same read in its
  commit message.

**`off_render_pass` (7597) stays at its statement.** It is allocated after `emit_vulkan_target`
(7581), which itself calls `allocate_stack_object` 10 times (`sed -n 2706,3273p … | grep -n
allocate_stack_object`, `vk_image_info` … `vk_buffer_info`).
- `emit_frame_setup` returns it as `Result<usize, String>` (tail `Ok(off_render_pass)`).
- R5 reads it at 7729 and receives it as a parameter.
- R6 and R7 do not use it (`grep -n off_render_pass` over 7941–8541 → empty).

**Locals that stay inside their phase:**
- R3: the `no_entry` label.
- R4: `first` (twice, in two scopes), `chosen`, `clear_slot`, `load_slot` and `load_layout`.
  `load_layout` depends on `present_mode`.
- R6: `same_mode`.
- R7: `end_recording`.

**Parameters that cross:**
- `platform: &dyn CodegenPlatform` and `platform_imports: &HashMap<String, String>` go to R4, R5
  and R7. R6 calls no dlsym (7941–8280 read).
- `present_mode: bool` goes to R4 (7598).
- The 8 operands (`surface`, `width`, `height`, `geometry`, `offsets`, `glyph_meta`,
  `glyph_coverage`, `draws`) go only to R3. `grep -n "width\.\|height\.\|draws\.\|geometry\.\|offsets\.\|glyph_meta\b\|glyph_coverage"`
  over 7543–8541 matches only the `off_*` names.

**The remaining fn:**

```rust
pub(crate) fn emit_vulkan_draw_scene(builder, platform, platform_imports, surface, width, height,
        geometry, offsets, glyph_meta, glyph_coverage, draws) -> Result<(), String> {
    let present_mode = surface.is_none();                       // R1 unchanged
    if !has_vulkan_backend(platform) || (present_mode && !present::has_present_path(platform)) {
        if present_mode { builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0")); }
        return Ok(());
    }
    let s = DrawSceneSlots::allocate(builder, present_mode);   // R2a + R2b + 2 hoisted slots
    emit_park_arguments(builder, &s, surface, width, height, geometry, offsets,
                        glyph_meta, glyph_coverage, draws);      // R3
    let off_render_pass = emit_frame_setup(builder, platform, platform_imports, &s, present_mode)?; // R4
    emit_record_begin(builder, platform, platform_imports, &s, off_render_pass)?;                  // R5
    emit_item_walk(builder, &s);                                                                    // R6
    emit_finish_frame(builder, platform, platform_imports, &s)                                      // R7 (tail)
}
```

The signature and R1 are kept verbatim. The sketch above abbreviates the parameter types; they
are the original ones.

**Renames inside the moved bodies:**
- `off_X` → `s.off_X` for every struct slot (`\boff_` → `s.off_` over the moved text). The
  exception is `off_render_pass`, a local in R4 and a parameter in R5.
- The 8 frame labels become `s.<name>` (`&unavailable` → `&s.unavailable`, `&done` →
  `&s.done`, …). The phase-local labels listed above keep their names.
- `bands` → `s.bands` (`&bands` → `&s.bands`, `bands.cursor` → `s.bands.cursor`).
- `swapchain` → `s.swapchain` (`&swapchain` → `&s.swapchain`).
- The struct literals `PictureSlots { state: off_state, … }` (8202) and `GlyphPublishSlots { … }`
  (8231) become `state: s.off_state`, and so on.
- R3 loses its two `let off_draw_entries = …` / `let off_pass = …` lines and reads
  `s.off_draw_entries` / `s.off_pass`.

**Order check.** Every `label()`, `allocate_stack_object()` and helper call (`emit_dlsym`,
`emit_vulkan_target`, `emit_load_render_pass`, `emit_ensure_swapchain`, `emit_picture_upload`,
`emit_glyph_publish`, `emit_draw_list_pass`, `present::*`) happens in the same order relative to
every other one.
- The phases are consecutive, and the only reorder is the proven hoist.
- `grep -c temporary_vreg` over 7308–8541 → 0.
- Nothing is cloned that was not cloned before.

**Files** (LOC are guesses; the census re-measures each in Phase 5):

| File | Items | Code LOC |
|---|---|---|
| `vulkan/draw_scene/mod.rs` | `use super::*;`, `mod record; mod finish; use record::{emit_record_begin, emit_item_walk}; use finish::emit_finish_frame;`, `struct DrawSceneSlots`, `impl DrawSceneSlots { fn allocate }`, `pub(crate) fn emit_vulkan_draw_scene`, `fn emit_park_arguments`, `fn emit_frame_setup` | 470 (guess: struct 60 + allocate 120 + main 45 + park 116 + setup 114 + header 6) |
| `vulkan/draw_scene/record.rs` | `use super::*;`, `pub(super) fn emit_record_begin`, `pub(super) fn emit_item_walk` | 550 (guess: 252+8 and 285+4) |
| `vulkan/draw_scene/finish.rs` | `use super::*;`, `pub(super) fn emit_finish_frame` | 245 (guess: 236+7) |

**Signatures:**
- `fn allocate(builder: &mut CodeBuilder, present_mode: bool) -> DrawSceneSlots`.
- `#[allow(clippy::too_many_arguments)] fn emit_park_arguments(builder: &mut CodeBuilder, s:
  &DrawSceneSlots, surface: Option<&Operand>, width: &Operand, height: &Operand, geometry:
  &Operand, offsets: &Operand, glyph_meta: &Operand, glyph_coverage: &Operand, draws: &Operand)`.
  It has 10 params, following the file's 10 existing `too_many_arguments` allows.
- `fn emit_frame_setup(builder: &mut CodeBuilder, platform: &dyn CodegenPlatform,
  platform_imports: &std::collections::HashMap<String, String>, s: &DrawSceneSlots,
  present_mode: bool) -> Result<usize, String>`.
- `pub(super) fn emit_record_begin(builder, platform, platform_imports, s: &DrawSceneSlots,
  off_render_pass: usize) -> Result<(), String>`, with tail `Ok(())`.
- `pub(super) fn emit_item_walk(builder: &mut CodeBuilder, s: &DrawSceneSlots)`.
- `pub(super) fn emit_finish_frame(builder, platform, platform_imports, s: &DrawSceneSlots) ->
  Result<(), String>`. The body is R7 verbatim, including the inner `return Ok(())` and the
  final `Ok(())`.

The largest fn afterwards is `emit_item_walk`, about 289 (guess). Phase 5 adds 6 new fns and 1
struct. A ±10 % rustfmt reflow from `off_x` → `s.off_x` still leaves every file under 900.

**Visibility.**
- `DrawSceneSlots` and its fields are private to `draw_scene` (`mod.rs`). `record` and `finish`
  are child modules, so they read the fields with no `pub`.
- The 3 child fns are `pub(super)`, the same reach as `DrawSceneSlots`, so
  `private_interfaces` cannot fire.
- Everything else arrives through `use super::*` (`draw_scene` → `vulkan`, whose private part
  globs are visible to descendants). That covers `emit_struct`, `emit_dlsym`,
  `emit_vulkan_target`, `BandSlots`, `PictureSlots`, `GlyphPublishSlots`, `emit_branch_on_pass`,
  `present::*` and the consts, using §4.3's `pub(super)` list unchanged.
- `BandSlots.cursor` is already in that list, and the draw scene reads only `bands.cursor` and
  passes `&bands`.

**Borrows.**
- `builder: &mut CodeBuilder` and `s: &DrawSceneSlots` are disjoint objects. `&s.unavailable`
  is held only for the duration of each helper call, as `&unavailable` was, and
  `if let Some(at) = &s.swapchain` borrows `s`, not `builder`.
- In `allocate`, the closure reborrows `builder` mutably as before.
  `BandSlots::allocate(builder, off_state, off_item, off_header)` takes the `usize`s by value
  before the struct exists.
- All of them are free fns, and none forces a clone.

**Citations.**
- `app/06_canvas.md:598` (the swapchain present path) →
  `[[src/codegen/runtime/canvas/vulkan/draw_scene/mod.rs:emit_vulkan_draw_scene]]`.
- `app/06_canvas.md:397` →
  `[[src/codegen/runtime/canvas/vulkan/draw_scene/finish.rs:emit_finish_frame]]`. That
  sentence claims "the Vulkan readback path does not test the results of `vkQueueSubmit` and
  `vkQueueWaitIdle`", and the submit/wait code now lives in `emit_finish_frame`. The claim
  holds: `sed -n 8352,8480p` shows the two `emit_call_fn` calls for them, and no compare
  follows either.
- The doc comment at `present/mod.rs` (orig `vulkan_present.rs:379`), "The stack slots the
  present path shares with `emit_vulkan_draw_scene`", stays true.

**Gate.**
- C1 (all four targets; the three Vulkan targets exercise this fn).
- `cargo test --test codegen_canvas_vulkan_off_frame_path`, which runs the emitted members.
- The unit tests under `codegen::runtime::canvas`.

### 4.6 `.ai`, comment and shader sweep

Every hit below names a file that no longer exists after V, or a `canvas/mod.rs` symbol that
moved. Each is repointed in the phase that moves the code it names.

The end state is checked by these two greps:
- `grep -rnE '(canvas/|`)vulkan(_present)?\.rs' src tests tools scripts .ai | grep -v '^src/docs/spec'`
  → empty.
- `grep -rn 'runtime/canvas/mod\.rs' src tests tools scripts .ai | grep -v '^src/docs/spec'`
  → empty.

Line numbers are as of a3c95fc07. Files that earlier letters split (`helper_render.rs` S,
`rt_canvas_font.rs` D, `helper_render/tests/` H) are found by the greps above, not by line.

| Phase | Where | Text now | Becomes |
|---|---|---|---|
| 2 | `shaders/mfb_canvas.frag:112` | `VULKAN_SPEC_CONSTANTS` in `runtime/canvas/mod.rs` | `runtime/canvas/scene_layout.rs` |
| 2 | `vulkan.rs:3702` (doc of `emit_item_block`) | layout and header slots "in `runtime/canvas/mod.rs`" | `runtime/canvas/scene_layout.rs` |
| 2 | `tests/codegen/codegen_canvas_vulkan_off_frame_path.rs:46` | `GRAPHICS_OFFSET_VULKAN_READY` (`src/codegen/runtime/canvas/mod.rs`) | `src/codegen/runtime/canvas/state_layout.rs` |
| 3 | `.ai/arch-abi.md:70` | `runtime/canvas/vulkan.rs:emit_int_arg_zero` | `runtime/canvas/vulkan/calls.rs:emit_int_arg_zero` |
| 3 | `.ai/canvas-threading.md:585` | `runtime/canvas/vulkan_present.rs` | `runtime/canvas/vulkan/present/` |
| 3 | `canvas/mod.rs:541` (now `scene_layout.rs`) | `the_item_block_matches_the_std430_stride` in `vulkan.rs` | in the `vulkan/tests/` file that holds it (`grep -rln the_item_block_matches_the_std430_stride src/codegen/runtime/canvas/vulkan/tests`) |
| 3 | `shaders/mfb_canvas.vert:17` | same test "in `vulkan.rs`" | same file as the row above |
| 3 | `shaders/mfb_canvas.frag:634` | `emit_band_index` in `vulkan.rs` | `vulkan/bands.rs` |
| 3 | `canvas/mod.rs:384` (now `state_layout.rs`) | (`vulkan_present.rs`) | (`vulkan/present/`) |
| 3 | `vulkan.rs:191` (now `vulkan/mod.rs`) | the swapchain present (`vulkan_present.rs`) | (`present/`) |
| 3 | `vulkan.rs:6746` (now `vulkan/publish.rs`) | `canvas::vulkanPresentScene` (`vulkan_present.rs`) | (`vulkan/present/`) |
| 3 | `vulkan_present.rs:128` (now `present/mod.rs`) | "the rest are in `vulkan.rs`" | "the rest are in `vulkan/mod.rs`" |
| 3 | `src/target/win_x86_64/app/mod.rs:207` | (`vulkan_present.rs`) | (`runtime/canvas/vulkan/present/`) |
| 3 | `src/target/macos_aarch64/app/metal.rs:4682` | `emit_glyph_publish` in `runtime/canvas/vulkan.rs` | `runtime/canvas/vulkan/publish.rs` |
| 3 | `metal.rs:5050` | the glyph-quad block in `runtime/canvas/vulkan.rs` | `runtime/canvas/vulkan/publish.rs` (`vk_glyph_hull`, orig 7159) |
| 3 | `metal.rs:5236` | the residual loop in `runtime/canvas/vulkan.rs` | `runtime/canvas/vulkan/item_block.rs` (`ITEM_RESIDUAL_FIELDS` loop, orig 3800) |
| 3 | `metal.rs:5424` | the cap-tag block in `runtime/canvas/vulkan.rs` | `runtime/canvas/vulkan/item_block.rs` (orig 3968) |
| 3 | `metal.rs:5572` | `emit_split_or_publish` in `runtime/canvas/vulkan.rs` | `runtime/canvas/vulkan/publish.rs` |
| 3 | `metal.rs:5726` | `emit_draw_list_pass` in `runtime/canvas/vulkan.rs` | `runtime/canvas/vulkan/publish.rs` |
| 3 | `metal.rs:5954` | `emit_gradient_upload` in `runtime/canvas/vulkan.rs` | `runtime/canvas/vulkan/item_block.rs` |
| 3 | `metal.rs:6898` | `emit_join_upload` in `runtime/canvas/vulkan.rs` | `runtime/canvas/vulkan/stroke_style.rs` |
| 3 | `src/codegen/builtins/canvas/gen_system_fonts_linux.rs:5` | Vulkan is (`runtime/canvas/vulkan.rs`) | (`runtime/canvas/vulkan/`) |
| 3 | `src/codegen/builtins/canvas/func_metal_draw.rs:185` | the whole emitter lives in `runtime/canvas/vulkan.rs` | `runtime/canvas/vulkan/` |
| 3 | `helper_render.rs` MFBASIC `'` comment (orig 519) | `emit_split_or_publish` and `emit_glyph_publish` in src/codegen/runtime/canvas/vulkan.rs | `src/codegen/runtime/canvas/vulkan/publish.rs` |
| 3 | `helper_render.rs` MFBASIC `'` comment (orig 589) | `emit_draw_list_pass` in src/codegen/runtime/canvas/vulkan.rs | `src/codegen/runtime/canvas/vulkan/publish.rs` |
| 3 | `helper_render/tests/…` (orig `mod.rs:225`) | `emit_picture_lookup` in `metal.rs` and `vulkan.rs` | `metal.rs` and `vulkan/picture.rs` |
| 3 | `tests/canvas/rt_canvas_font…` (orig `rt_canvas_font.rs:1557`) | the `_glyph_hull` block in `runtime/canvas/vulkan.rs` | `runtime/canvas/vulkan/publish.rs` |
| 4 | `.ai/canvas-threading.md:585` | `runtime/canvas/vulkan/present/` | add ``; record-and-present in `vulkan/present/submit.rs` `` |

The two `helper_render.rs` rows edit MFBASIC comment text inside an embedded helper, on the same
lines. No span or line number changes. C1 and the full artifact gate prove the IR is unmoved.

## Phases

> **NOTE — keep the checkboxes current as you go. This is not bookkeeping; it is
> the only way anyone can see where the work actually is.**
>
> - Tick `- [x]` **in the same commit as the work it describes** — never in a
>   batch at the end of a phase, never "I'll update the plan after". If it is a
>   separate step it will be skipped, and the list becomes a lie.
> - Use `- [~]` for partially done and say in one line what remains.
> - Mark a task moot with `- [x] ~~text~~ — moot: <evidence>` rather than
>   deleting it. A vanished task is indistinguishable from a forgotten one.
> - Fill the phase's `Commit:` line with the hash(es) the moment it lands.
> - Add any task you discover you needed; an unlisted task that got done is a
>   task the next reader will do twice.
>
> **An unticked box means NOT DONE.** Someone reading this file — including the
> person who has to decide whether to stop — is entitled to trust that.

**Common per-phase check sequence.** Each phase's Acceptance names its scoped gates, test
filters and expected rows.

1. Before editing, record two baselines:
   - N: `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'`
   - W: `cargo check --all-targets 2>&1 | grep -c '^warning'`
2. After editing, run `rustfmt --edition 2021 <each new/changed .rs file>`. `git diff --stat`
   must show only this phase's files.
3. `cargo check --all-targets 2>&1 | grep -c '^warning'` → W, so no new warning (est. 2 min).
4. `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` → N (est. 2 min).
5. `cargo test -p mfb --bin mfb spec_citations_resolve` → passes (est. 2 min).
6. `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings` →
   pass (est. 2 min).
7. `cargo build --release` (est. 5 min, guess), then the phase's scoped gates. Before any gate,
   `pgrep -f artifact-gate` must be empty: no two gates run at once (`.ai/testing-gates.md`).
   C1 builds in the fixture directory just as the gate does.
   - **C1**, the canvas golden on all four targets (est. 2 min, guess). Expected: four `SAME`
     lines.
     ```
     ( cd tests/syntax/app/app-mouse-surface && for t in macos-aarch64 linux-x86_64 linux-aarch64 windows-x86_64; do tg=""; [ "$t" = macos-aarch64 ] || tg="-target $t"; ../../../../target/release/mfb build -q -ncode $tg --app . >/dev/null 2>&1; got=$(shasum -a 256 app_mouse_surface.ncode | cut -d' ' -f1); want=$(cut -d' ' -f1 golden/app_mouse_surface.$t.app.ncodesum); [ "$got" = "$want" ] && echo "SAME $t" || echo "DIFF $t"; rm -f app_mouse_surface.ncode; done )
     ```
   - The thread phase runs `sh scripts/artifact-gate.sh target/release/mfb <builtin>` → 0 diffs
     (~1 min each).
8. `grep -rn GRAPHICS_OFFSET_VULKAN src/target` → empty (§10.1, est. 5 s).
9. `cargo test --test file_size_budget 2>&1 | grep -E 'src/codegen/runtime/(canvas|thread)/'` →
   only the rows the phase names. Then run `MFB_LOC_TIGHTEN=1 cargo test --test
   file_size_budget` and commit `tests/guards/file_size_ceilings.txt` with the phase
   (est. 1 min). Tighten removes the `ceiling` rows of the old paths as missing files.
10. For every new file, `python3 -I tools/loc-census/census.py --file <path>` → code ≤ 900.

### Phase 1 — Thread runtime helpers (pure motion)

- [x] `git mv src/codegen/runtime/thread/runtime_helpers_thread.rs
      src/codegen/runtime/thread/runtime_helpers_thread/mod.rs`, then split it into
      `{mod,handle,sleep,queue}.rs` per §4.1: the a3c95fc07 ranges, `mod` decls and the three
      `pub(crate) use <part>::*;`. There are 0 visibility edits.
- [x] Repoint the 21 spec citations per the §4.1 table (7 spec files).

Acceptance: the thread helpers compile from their new layout with every outside path unchanged,
the code they emit is byte-identical, and the moved emitters still run real thread programs.
  Check: common sequence, with scoped gates `sh scripts/artifact-gate.sh target/release/mfb
  thread` and `… os` → 0 diffs each (~1 min each).
  Then `cargo test --test rt_thread_exit_program_in_worker --test
  rt_thread_drop_running_finish_race --test rt_thread_block_arena` → pass (est. 3 min, guess).
  Then the citation counts:
  - `grep -rnoE '\[\[src/codegen/runtime/thread/runtime_helpers_thread\.rs:' src/docs/spec | wc
    -l` → 0;
  - `grep -rnoE '\[\[src/codegen/runtime/thread/runtime_helpers_thread/(mod|handle|sleep|queue)\.rs:'
    src/docs/spec | wc -l` → 21.

  Step 9 shows no `runtime/thread/` row. The ceiling row for `runtime_helpers_thread.rs` is
  removed by tighten (est. 15 min total).
  Done: plain `mv` to `runtime_helpers_thread/mod.rs` (the U archive was committed while this ran; git records the
  rename at commit), then `codesplit.py` by name (`v1-thread.json`); 0 visibility edits, as §4.1 says.
  Verified 2026-10-20: `motion.py` → only the six wiring lines differ; N = 5018, W = 0; `spec_citations_resolve` ok;
  guards → 9, 8, 7 + 1 ignored; artifact gates `thread`, `os` → 0 diffs each; `rt_thread_exit_program_in_worker` 1,
  `rt_thread_drop_running_finish_race` 1, `rt_thread_block_arena` 5 → all passed; cite counts → 0 old, 21 new; §10.1
  grep → empty; budget → the one STALE row, gone after the tighten; census → mod 237, handle 529, sleep 150,
  queue 455.
Commit: 676b92ded

### Phase 2 — Canvas runtime root (pure motion, path kept)

- [x] Split `src/codegen/runtime/canvas/mod.rs` into `mod.rs` +
      `{state_layout,scene_layout,entry,thread_lifecycle,frame_sync,surface}.rs` per §4.2,
      with the `mod` decls and the `pub(crate) use <part>::*;` re-exports. Trim `mod.rs`'s
      `use` lines.
- [x] `pub(super)` on `emit_frame_cond_broadcast`, `NANOS_PER_SECOND` and `emit_realtime_nanos`
      (in `frame_sync.rs`).
- [x] Repoint the 47 `canvas/mod.rs` citations per the §4.2 table (`app/06_canvas.md`,
      `tooling/01_project-manifest.md`, `tooling/09_debug-report.md`).
- [x] The three Phase-2 rows of §4.6: `mfb_canvas.frag:112`, the `emit_item_block` doc comment
      in `vulkan.rs` and `codegen_canvas_vulkan_off_frame_path.rs:46`. No `.spv` is
      regenerated (§2, `-V` without `-g`).

Acceptance: every canvas caller resolves unchanged, the canvas runtime emits byte-identical
code on all four targets, the graphics thread's entry and frame sync still run, and no spec
citation still points at a symbol that left `mod.rs`.
  Check: common sequence, with scoped gate C1 → `SAME` ×4.
  Then the tests:
  - `cargo test -p mfb --bin mfb codegen::runtime::canvas` → the 6 `canvas/mod.rs` tests and
    the rest pass;
  - `cargo test --test rt_canvas_graphics_thread --test codegen_canvas_thread_entry_saves_rbp
    --test rt_canvas_frame_stats --test rt_canvas_pointer` → pass (est. 6 min, guess).

  Then `grep -rnoE '\[\[src/codegen/runtime/canvas/mod\.rs:' src/docs/spec | wc -l` → 0, and
  `grep -rn 'runtime/canvas/mod\.rs' src tests tools scripts .ai | grep -v '^src/docs/spec'` →
  empty.

  Step 9 shows no row for `canvas/mod.rs` or its six siblings, and tighten removes the
  `canvas/mod.rs` ceiling row because the file is now within its limit (est. 20 min total).
- [x] `tests/codegen/codegen_canvas_metal_off_frame_path.rs:48` (`GRAPHICS_OFFSET_MTL_READY` in `canvas/mod.rs`) →
      `state_layout.rs`: the Metal twin of the `:46` Vulkan row, not in §4.6's table (`grep -rn
      'runtime/canvas/mod\.rs' tests`).
  Done: `v2.py` split by line range at the six blank-line boundaries (state_layout 78–508, scene_layout 510–1734,
  entry 1736–2145, thread_lifecycle 2147–2873, frame_sync 2875–3654, surface 3656–4086 of the old file; mod.rs
  keeps 1–77 and the tests 4088–4316); `pubsuper.py` made the 3 §4.2 items `pub(super)`; `state_layout.rs` takes no
  `use super::*;` (it reads nothing from the parent: `unused_imports`).
  Verified 2026-10-20: `motion.py` → only the wiring and the 3 `pub(super)` differ; N = 5018, W = 0;
  `spec_citations_resolve` + `codegen::runtime::canvas` → 26 passed; guards → 9, 8, 7 + 1 ignored; C1 → `SAME` ×4;
  `rt_canvas_graphics_thread` 18 (+2 ignored, pre-existing), `codegen_canvas_thread_entry_saves_rbp` 2,
  `rt_canvas_frame_stats` 13, `rt_canvas_pointer` 2 → all passed; cites → 49 repointed (2 added since planning),
  `[[…/canvas/mod.rs:` → 0; the `runtime/canvas/mod.rs` grep outside the spec → only the new part headers and the
  ceiling row the tighten then removed; §10.1 grep → empty; budget → `TIGHTEN code 44 (ceiling 2272)`, removed;
  census → mod 44 + 178 test, state_layout 124, scene_layout 414 + 2 test, entry 195, thread_lifecycle 569,
  frame_sync 669, surface 274 + 21 test.
Commit: e9259dfee

### Phase 3 — `vulkan.rs` → `vulkan/`, `vulkan_present.rs` → `vulkan/present/mod.rs` (pure motion)

- [x] `git mv src/codegen/runtime/canvas/vulkan.rs src/codegen/runtime/canvas/vulkan/mod.rs`.
      `vulkan/tests/` from J is already in that directory.
- [x] `git mv src/codegen/runtime/canvas/vulkan_present.rs
      src/codegen/runtime/canvas/vulkan/present/mod.rs`. In `vulkan/mod.rs`, replace
      `#[path = "vulkan_present.rs"]` + `mod present;` with `mod present;`.
- [x] `include_bytes!` paths → `"../shaders/mfb_canvas.vert.spv"` /
      `"../shaders/mfb_canvas.frag.spv"`.
- [x] Split `vulkan/mod.rs` into the 14 parts of §4.3 (`calls`, `memory`, `build`,
      `descriptors`, `pipeline`, `target`, `item_block`, `edges`, `stroke_style`, `bands`,
      `picture`, `publish`, `draw_scene`, `passes`). Add the 14 `mod` decls, the 12 private
      part globs and the two explicit `pub(crate) use` re-exports.
- [x] The 62 `pub(super)` additions of §4.3 (39 items, 23 fields).
- [x] Repoint the 14 `vulkan.rs` citations and the 4 `vulkan_present.rs` citations
      (→ `vulkan/present/mod.rs`) per §4.3.
- [x] `.ai/arch-abi.md:70`, `.ai/canvas-threading.md:585`, and every Phase-3 row of the §4.6
      table.

Acceptance: both Vulkan modules compile from the new tree with no `#[path]`, every outside caller
resolves unchanged, the Vulkan and Metal code emitted on all four targets is byte-identical, and
the emitted Vulkan members still run in the interpreter.
  Check: common sequence, with scoped gate C1 → `SAME` ×4.
  Then the tests:
  - `cargo test -p mfb --bin mfb codegen::runtime::canvas` → the 8 `vulkan::tests` (J's), the 2
    `present` tests and the 6 canvas tests pass;
  - `cargo test --test codegen_canvas_vulkan_off_frame_path` → pass (est. 4 min, guess).

  Then the greps:
  - `grep -rn '#\[path' src/codegen/runtime/canvas` → empty;
  - `grep -rnoE '\[\[src/codegen/runtime/canvas/vulkan(_present)?\.rs:' src/docs/spec | wc -l`
    → 0;
  - `grep -rnE '(canvas/|`)vulkan(_present)?\.rs' src tests tools scripts .ai | grep -v
    '^src/docs/spec'` → empty.

  Step 9 shows exactly two rows, the expected intermediates of §3:
  - `NEW code 1060 > 1000 src/codegen/runtime/canvas/vulkan/draw_scene.rs`
  - `NEW code 1848 > 1000 src/codegen/runtime/canvas/vulkan/present/mod.rs`

  Tighten removes the `vulkan.rs` and `vulkan_present.rs` ceiling rows as missing paths. Every
  other new `vulkan/` file is ≤ 900 by step 10 (est. 25 min total).
- [x] Repoint the comment mentions §4.6's table did not list: `vulkan/publish.rs` (old 6746's neighbour, the
      read-back note naming `vulkan_present.rs`) and `metal.rs`'s draw-list twin (`emit_draw_list_pass`, old
      5726) were in the table; the rest of the 25 hits are its rows at shifted lines (`grep -rnE
      '(canvas/|`)vulkan(_present)?\.rs|vulkan\.rs|vulkan_present\.rs' src tests tools scripts .ai`).
  Done: `git mv` both files, then `codesplit.py` by item name (`v3-vulkan.json`; every name §4.3 lists, at lines
  ~25 later than planned); the wiring script removed the `#[path]`, re-based both `include_bytes!` paths, and
  wrote the 14 `mod` lines, the two `pub(crate) use` re-exports and the 12 private globs. Visibility: `autovis.py`
  added 38, then `BandSlots.cursor` (a use-site error `autovis` does not parse), `BandSet` (`private_interfaces`)
  and all fields of `PictureSlots` (6) and `GlyphPublishSlots` (16) by hand → 62 new `pub(super)` (63 in `vulkan/`
  minus the existing `emit_branch_on_pass`), exactly §4.3's count. `present/mod.rs` unchanged but for one comment.
  Verified 2026-10-20: `motion.py` (ignoring `pub(super)`) → only wiring, the `#[path]`/`include_bytes!` edits and
  rustfmt re-wraps; N = 5018, W = 0; `spec_citations_resolve` + `codegen::runtime::canvas` → 26 passed; guards →
  9, 8, 7 + 1 ignored; C1 → `SAME` ×4; `codegen_canvas_vulkan_off_frame_path` → 6 passed; cites → 14 + 4
  repointed; the `#[path]` grep, the old-cite grep, the comment grep and §10.1 → empty/0; budget → `NEW code 1061 >
  1000 …/vulkan/draw_scene.rs` (1061, not 1060) and `NEW code 1848 > 1000 …/vulkan/present/mod.rs`, both STALE rows
  gone after the tighten; census → mod 523 + 33 test, calls 338, memory 255, build 577, descriptors 504, pipeline
  586, target 541, item_block 440, edges 547, stroke_style 344, bands 441, picture 345, publish 753, passes 114.
Commit: —

### Phase 4 — `vulkan/present/` parts (pure motion)

- [ ] Split `vulkan/present/mod.rs` into `{mod,wsi,swapchain,render_pass,teardown,submit}.rs`
      per §4.4, with the `mod` decls, the `pub(super) use self::{…}` re-export and the two
      private imports.
- [ ] The 7 visibility edits of §4.4: five to `pub(in crate::codegen::runtime::canvas::vulkan)`
      and two to `pub(super)`.
- [ ] Repoint the 3 citations per §4.4 (`emit_submit_and_present` ×2 → `submit.rs`,
      `emit_ensure_swapchain` → `swapchain.rs`).
- [ ] `.ai/canvas-threading.md:585`: the Phase-4 row of §4.6.

Acceptance: the Windows swapchain present path emits byte-identical code, and its layout tests
still pass.
  Check: common sequence, with scoped gate C1 → `SAME` ×4. `windows-x86_64` is the target that
  emits the present path (`has_present_path`).
  Then `cargo test -p mfb --bin mfb codegen::runtime::canvas::vulkan::present` → the 2 tests
  pass (est. 2 min).
  Step 9 shows exactly one row,
  `NEW code 1060 > 1000 src/codegen/runtime/canvas/vulkan/draw_scene.rs`. Step 10 shows
  `swapchain.rs` at 844 and every other `present/` file under it (est. 15 min total).
Commit: —

### Phase 5 — Extract `emit_vulkan_draw_scene` (largest blast radius last)

- [ ] Record in the commit message the §4.5 hoist read: `vulkan/draw_scene.rs` between the last
      R2b allocation and the `vk_pass_word` allocation (orig 7405–7483) holds only
      `builder.emit(...)` calls.
- [ ] `git mv src/codegen/runtime/canvas/vulkan/draw_scene.rs
      src/codegen/runtime/canvas/vulkan/draw_scene/mod.rs`. Then add `DrawSceneSlots` and
      `DrawSceneSlots::allocate` (R2a + R2b + the two hoisted slots), `emit_park_arguments`
      (R3) and `emit_frame_setup` (R4, returning `off_render_pass`) in `mod.rs`.
- [ ] `vulkan/draw_scene/record.rs`: `emit_record_begin` (R5) and `emit_item_walk` (R6).
      `vulkan/draw_scene/finish.rs`: `emit_finish_frame` (R7, the tail call).
- [ ] Apply the §4.5 renames (`s.off_*`, `s.<label>`, `s.bands`, `s.swapchain`, the two struct
      literals). The remaining `emit_vulkan_draw_scene` matches the §4.5 sketch.
- [ ] Repoint `app/06_canvas.md:598` → `vulkan/draw_scene/mod.rs:emit_vulkan_draw_scene` and
      `:397` → `vulkan/draw_scene/finish.rs:emit_finish_frame`.

Acceptance: no function under `src/codegen/runtime/` is over 1000 code LOC, and the frame
emitter's output is byte-identical on every target. A reordered allocation or label would move
every later slot offset or label number and show up as a `DIFF` on all three Vulkan targets.
  Check: common sequence, with scoped gate C1 → `SAME` ×4.
  Then the tests:
  - `cargo test --test codegen_canvas_vulkan_off_frame_path` → pass;
  - `cargo test -p mfb --bin mfb codegen::runtime::canvas` → pass (est. 6 min, guess).

  Then `cargo clippy --all-targets 2>&1 | grep -c 'draw_scene'` → 0 (est. 3 min). Step 9 shows
  no `runtime/canvas/` or `runtime/thread/` row. Step 10 shows each of
  `draw_scene/{mod,record,finish}.rs` ≤ 900 (est. 20 min total).
Commit: —

## Validation Plan

This is the plan-176 final gate, run ONCE after Phase 5, on the commit that lands V.

- **Tests:** no test is added or removed by V. The canvas unit tests (6 root, 2 `present`, 8 J
  `vulkan::tests`) run in Phases 2–5. The `mfb` binary's test count is equal before and after
  every phase.
- **Full workspace suite:** `cargo build --release` (est. 5 min, guess), then `cargo test
  --workspace --no-fail-fast` → 0 failures (est. 60–90 min, guess). This is the one full-suite
  run for all of plan-176 (the brief). B–V moved tests, code and fixtures across every test
  target, and only the full suite proves that no target lost a test or an include path.
- **Full artifact gate:** `sh scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)`
  (15–20 min). This is the only gate that covers every fixture on every target, including
  `tests/syntax/app/app-mouse-surface`. Letters L–V moved codegen whose output any fixture
  can see.
- **Canvas GPU checks** (by hand, one at a time, each on the box `scripts/README.md` and
  `.ai/remote_systems.md` assign it):
  - `bash scripts/test-canvas-vulkan.sh target/release/mfb` runs on box 2228 (linux-x86_64
    glibc, lavapipe) → every scene agrees with the software oracle within tolerance, including
    the resize, group, polyline and particle scenes (est. 10 min, guess). This runs
    `emit_vulkan_build`/`ready`, the descriptors and pipeline, and every upload and the draw
    scene on a real driver.
  - `bash scripts/test-canvas-vulkan.sh target/release/mfb --box 2226 --target linux-aarch64`
    runs on box 2226 (aarch64, lavapipe) → as above (est. 10 min, guess). This covers the
    AArch64 store-release/load-acquire ready-word order of §10.1.
  - `bash scripts/test-canvas-gpu-rows.sh target/release/mfb --target linux-aarch64` runs on box
    2226 → every row draws on the GPU (`gpuFrames` non-zero) and agrees with the oracle (est.
    15 min, guess). This covers the scale rows, the bands, the pictures and the multi-pass
    frames.
  - `bash scripts/test-canvas-gpu-rows.sh target/release/mfb --target windows-x86_64` runs on
    box 206, port 22 by default, the only real GPU → as above (est. 15 min, guess).
  - `bash scripts/test-winapp.sh target/release/mfb --box 22` runs on box 206 (2230 is retired,
    `.ai/remote_systems.md`) → every case passes, including the canvas Vulkan frames (est. 15
    min, guess). This is the Windows swapchain present path, `vulkan/present/`, in a real
    window.
- **macOS canvas path:** `cargo test --test rt_canvas_metal` is part of the workspace suite and
  covers the Metal side of `canvas/mod.rs`'s shared layout.
- **File-size end state:**
  - `cargo test --test file_size_budget 2>&1 | grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`
    (est. 1 min).
  - `grep -vcE '^(#|$)' tests/guards/file_size_ceilings.txt` → `0`, so the ceiling file holds
    no `ceiling code|inline|test` row (there is no `exempt` row format; rule revised by the
    owner, 2026-10-18). At planning (HEAD ce7a995ca, `python3 -I /tmp/loc/census.py "$PWD"`,
    /tmp/loc/newrule.json) plan-176 started from 98 `code` > 1000, 71 `inline` > 250 and 30
    `test` > 1000 files; all are gone.
  - `python3 -I tools/loc-census/census.py --seed | grep -c '^ceiling'` → `0`, so the oracle
    agrees (est. 30 s).
- **Doc sync:** no `mfb spec` or `mfb man` contract changes. Only citations move:
  `app/06_canvas.md`, `tooling/01`, `tooling/09`, `threading/06`, `07`, `08`, `09`, `11`,
  `language/08` and `language/15`, checked by `spec_citations_resolve` and by the
  `canvas/mod.rs` grep. `.ai/arch-abi.md` and `.ai/canvas-threading.md` are edited in the same
  phases as the code.
- **Coverage:** canvas code is outside `tests/byte-identity/` coverage by design
  (`.ai/testing-gates.md`). It is covered by C1 (change sentinel), the `rt_canvas_*` suites in
  the workspace run, and the GPU box scripts above.

## Decisions

- Rule revised. Owner, 2026-10-18 (replaces the 2026-10-14 "code ≤ 1000, test ≤ 250" rule).
  Three kinds, non-comment non-blank LOC, scope `.rs`/`.mfb` under `src/`, `packages/`,
  `examples/`, `repository/`, `tests/`: `code` ≤ 1000 (code LOC of a file that is not test-only;
  over → split into modules); `inline` ≤ 250 (`#[cfg(test)]` / `TESTING … END TESTING` LOC inside
  a file that is not test-only; over → moved to a test-only file, one `tests/mod.rs` unless
  > 1000); `test` ≤ 1000 (total LOC of a test-only file; over → split). Parts target ≤ 900. No
  exemptions: only `GENERATED` files are exempt automatically. Ceiling rows are
  `ceiling <code|inline|test> <n> <path>`; the guard warns with a banner and never fails; a new
  module is always an acceptable split. In V, `canvas/mod.rs` (178 inline test LOC in `mod.rs`,
  the other 23 travel with `surface.rs` and `scene_layout.rs`) and `present/mod.rs` (164) keep
  their inline tests, each under 250. `vulkan.rs`'s 748 inline test LOC moved in J to the one
  test-only file `vulkan/tests/mod.rs`.
- A single function over the code limit is decomposed by extract-method refactoring, gated by
  byte-identical `.ncode`. Owner, 2026-10-14. In V that is `emit_vulkan_draw_scene` (§4.5),
  following `/tmp/loc/maps/V-extract.md`, which supersedes V.md's draw-scene exemption. Nothing
  in V is exempt.
- `vulkan_present.rs` moves to `src/codegen/runtime/canvas/vulkan/present/` (`mod.rs` +
  `wsi`, `swapchain`, `render_pass`, `teardown`, `submit`) when `vulkan.rs` becomes
  `vulkan/mod.rs`. The `#[path]` attribute is removed, and the module is an ordinary `mod
  present;`. Author, 2026-10-14. Phase 3 moves it whole and Phase 4 splits it, so each commit
  has one gate-able change.
- `assert_layout` stays in `vulkan/mod.rs` (cfg(test)), because `present`'s test module calls it
  through `use super::*`. This is a prerequisite on plan-176-J's end state. Author.
- Private part globs in `vulkan/mod.rs` exist only for parts holding a `pub(super)` item. Author
  (§3, measured by `/tmp/loc/probe3`).
- The five `present` fns `vulkan` calls are `pub(in crate::codegen::runtime::canvas::vulkan)`.
  Author (§3).
- `DrawSceneSlots` carries the 58 shared labels and slots. `vk_draw_entries` and `vk_pass_word`
  are hoisted into `allocate`, and `off_render_pass` is not hoisted. Author (§4.5 proof).
- `app/06_canvas.md:397` cites `emit_finish_frame`, where the unchecked submit/wait now lives.
  Author.
- Every stale prose, shader and MFBASIC-comment reference to the four old paths is repointed
  (§4.6). Owner rule (AGENTS.md, never leave a bug), applied by the author.
- V's Validation Plan carries the plan-176 final gate: the full workspace suite, the full
  artifact gate, the canvas GPU box scripts, and an empty ceiling file. The brief, 2026-10-14;
  end state revised for the owner's 2026-10-18 rule (no `exempt` rows exist).

## Corrections

- 2026-10-18: Rule revised by the owner (code ≤ 1000, inline ≤ 250, test-only file ≤ 1000, no exemptions); file list and split design unchanged; rule text (intro, Goal, Decisions) restated, the 250 test-side `exempt` decision deleted, final gate now expects an empty ceiling file (was 20 `exempt` rows), J's vulkan tests named as the single `vulkan/tests/mod.rs`.

## Summary

The risk is in Phase 5, which changes a function boundary in the Vulkan frame emitter. It is
safe because:
- it moves verbatim bodies to their exact call positions;
- it keeps every allocation and label in its original order, the one hoist being proven by
  reading the code between;
- the two `return`s and every `?` keep their meaning;
- byte-identical `.ncode` on three Vulkan targets and the interpreter test prove it.

Phases 1–4 are pure motion. Their hazards are bookkeeping, and each is caught by
`cargo check`, `spec_citations_resolve` or an explicit grep:
- 86 spec citations;
- 27 prose, shader and `.ai` references;
- one `#[path]` removal;
- 72 visibility edits;
- the silent-pass trap of `canvas/mod.rs` keeping its path.

The threading invariants of `.ai/canvas-threading.md` §10.1 are preserved by construction:
- builder-thread code stays in `build`/`descriptors`/`pipeline`;
- the swapchain stays in `present/` on the graphics thread;
- `vulkan_handoff.rs` is untouched;
- nothing moves to `src/target`.

V closes plan-176 by running its final gate. Left untouched:
- every emitted byte, every crate path callers use, every test name;
- `vulkan_handoff.rs`, `curve_shader.rs`, `metal.rs`, `shaders/*.spv`;
- the bug and planning documents.

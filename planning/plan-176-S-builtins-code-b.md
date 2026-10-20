# plan-176-S: Built-ins code, part B (canvas, crypto, fs, math, money)

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-R

S splits the ten over-budget code files of `src/codegen/builtins/` in the S list: the canvas
render source (`canvas/helper_render.rs`) and package registrar (`canvas/mod.rs`), the three
EC crypto lowerings (`crypto/func_generate.rs`, `func_sign.rs`, `func_verify.rs`), three fs
helper carriers (`fs/gen_atomic_write.rs`, `gen_directory.rs`, `gen_read_write.rs`), the float
math carrier (`math/gen_math.rs`) and the fixed-point math kernels (`money/gen_fixed_math.rs`).
Eight files are pure code motion: each becomes a directory module whose `mod.rs` keeps the
surface. `helper_render.rs`'s one oversize item, the 1464-LOC `RENDER_METAL_TEMPLATE` raw
string, is rebuilt from three `macro_rules!` pieces with `concat!`, the precedent this file
already uses for `RENDER_LOOP`. One function is over the 1000-LOC code limit on its own:
canvas `register` (2093). It is decomposed by extract-method refactoring into five ordered
sub-registrars, in its own phase, after every pure-motion phase. Every phase is gated by
byte-identical codegen.

Single outcome: after S lands, `cargo test --test file_size_budget` prints no row for any of
the ten files or the files they become, and `tests/guards/file_size_ceilings.txt` has no
`ceiling` row for any of them. The rule (owner, 2026-10-18): code ≤ 1000, inline test ≤ 250,
test-only file ≤ 1000 non-comment non-blank LOC; there are no exemptions.

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, §Decisions, the guard, and the
  "Splitting an over-budget file (plan-176)" conventions in `.ai/build-tooling.md`.
- `.ai/compiler.md`, `.ai/codegen-invariants.md` — vreg/label allocation order (the impl
  splits must not reorder an emission call; pure motion cannot).
- `.ai/testing-gates.md` — the artifact gate; canvas is NOT reached by any
  `tests/byte-identity/<builtin>` selector, only by `tests/syntax/app/app-mouse-surface` in the
  full gate; `crypto` is also covered by `tests/rt-behavior/crypto/crypto-ec-valid`; never run
  two gates at once.
- `.ai/canvas-threading.md` — names `helper_render.rs` at two places this letter repoints (§4.11).
- `.ai/build-tooling.md` — rustfmt policy (`rustfmt --edition 2021 <files>` for scoped formatting).
- Spec topics whose citations move: `app/06_canvas.md`, `tooling/09_debug-report.md`,
  `stdlib/21_fs-host-errors.md`, `stdlib/23_fs-memory-files.md`,
  `architecture/18_math-kernels.md`, `stdlib/07_math-rng.md`.
- Split maps used to write this plan: `/tmp/loc/maps/S.md`, `/tmp/loc/maps/S-extract.md`
  (canvas `register`; its corrected ranges are used below).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-R complete and archived | `ls planning/plan-176-R-* planning/completed/plan-176-R-*` → only `planning/completed/…` | MET (2026-10-20) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | MET (2026-10-20) |
| S's files have no uncommitted changes | `git status --porcelain -- src/codegen/builtins/canvas src/codegen/builtins/crypto src/codegen/builtins/fs src/codegen/builtins/math src/codegen/builtins/money src/codegen/builtins/audio/func_render.rs src/codegen/builtins/tests src/codegen/collection/assign src/codegen/runtime/canvas/mod.rs src/codegen/string/format/float_parse_table.rs src/codegen/engine/tests src/docs/spec/app/06_canvas.md src/docs/spec/tooling/09_debug-report.md src/docs/spec/stdlib/21_fs-host-errors.md src/docs/spec/stdlib/23_fs-memory-files.md src/docs/spec/architecture/18_math-kernels.md src/docs/spec/stdlib/07_math-rng.md .ai/canvas-threading.md tests/runtime/rt_debug_soak.rs tests/canvas/rt_canvas_pointer.rs tests/guards/file_size_ceilings.txt` → empty | MET (2026-10-20: 0 lines) |

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

- Each of the ten files is replaced by a directory module (`canvas/mod.rs` gains five sibling
  files) whose every file is ≤ 900 code LOC by `python3 -I tools/loc-census/census.py --file <path>`.
- No function in S's files is over 1000 code LOC. Canvas `register` is decomposed exactly as
  §4.10 specifies; the largest function afterwards is `types_items::register` (≈504 LOC, guess
  from region counts; re-measured in Phase 5).
- `RENDER_METAL_TEMPLATE`'s `&'static str` is byte-identical, and `.ncode` (and every other
  artifact-gate dump kind) is byte-identical on every target before and after each phase.
- Every `[[…]]` spec citation, `.ai/` reference and code comment that names one of the ten old
  paths names the file that now holds the cited symbol.

### Non-goals (explicit constraints)

- No behavior change and no change to emitted code or to the canvas companion source on any target.
- No logic edit except the decided extraction (§4.10) and the decided `concat!` rebuild (§4.1).
  Pure-motion phases move text verbatim; the only other edits are `pub(super)` visibility,
  `use` lines, `mod` declarations, re-exports and comment repoints.
- No crate path that callers use changes: `canvas::register`, `crypto::func_*`,
  `fs::gen_*::lower_fs_*`, `math::gen_math::*` (through `math/mod.rs`'s `pub(crate) use gen_math::*;`)
  and `money::gen_fixed_math` methods all resolve unchanged.
- No registration order changes: the 45 `pkg.add_*` type statements and the 64 sub-registrar
  calls run in their original order.
- No test fn renamed, no test module renamed, no `[[test]]` target renamed. The unit-test
  count of the `mfb` binary is unchanged by every phase.
- No guard edited or weakened. The overflow-label scan already covers `gen_*` directories since
  plan-176-O Phase 1 (§3).
- `bugs/*.md` that name the old paths are historical records and are not edited.

## 2. Current State

### Files (as of 17a9b5799; unchanged at a3c95fc07)

LOC from `/tmp/loc/groups.json` (planning census `python3 -I /tmp/loc/census.py "$PWD"`), re-measured
on 2026-10-17 with `python3 -I /tmp/loc/span.py <file>` → same values. After A the command is
`python3 -I tools/loc-census/census.py --file <path>`. `git diff --stat 17a9b5799 HEAD -- <the ten files>`
→ empty, so every line range below is valid at HEAD.

| File | Kind | Code LOC | Test LOC | `wc -l` |
|---|---|---|---|---|
| `src/codegen/builtins/canvas/helper_render.rs` | code | 2183 | 2 (`mod tests;`) | 2425 |
| `src/codegen/builtins/canvas/mod.rs` | code | 2307 | 408 at planning; moved to `canvas/tests/mod.rs` by plan-176-H | 3132 |
| `src/codegen/builtins/crypto/func_generate.rs` | code | 1897 | 0 | 2085 |
| `src/codegen/builtins/crypto/func_sign.rs` | code | 1635 | 0 | 1808 |
| `src/codegen/builtins/crypto/func_verify.rs` | code | 1487 | 0 | 1684 |
| `src/codegen/builtins/fs/gen_atomic_write.rs` | code | 1197 | 0 | 1359 |
| `src/codegen/builtins/fs/gen_directory.rs` | code | 1028 | 0 | 1151 |
| `src/codegen/builtins/fs/gen_read_write.rs` | code | 1657 | 0 | 1864 |
| `src/codegen/builtins/math/gen_math.rs` | code | 1136 | 0 | 1412 |
| `src/codegen/builtins/money/gen_fixed_math.rs` | code | 1334 | 60 (1 test, stays inline) | 1812 |

State left by earlier letters:
- plan-176-H moves `canvas/mod.rs`'s inline `mod tests` (lines 2551-3132, 14 tests) to
  one test-only file `canvas/tests/mod.rs` (≤ 1000, so not split), leaves `#[cfg(test)] mod tests;`
  at the end of `canvas/mod.rs`, and repoints the three test-symbol citations
  (`draw_item_variant_set_is_frozen`, `no_canvas_type_reaches_itself`,
  `canvas_text_has_one_glyph_walk`). Lines 1-2550 are unchanged by H. S does not touch the tests.
- `canvas/helper_render/tests/mod.rs` is a test-only file of 308 LOC (≤ 1000), so no letter splits
  it; `helper_render.rs`'s `#[cfg(test)] mod tests;` resolves to it, and stays so after S.
- No earlier letter edits the other eight files (`grep -l` over `/tmp/loc/maps/[A-R]*.md` for
  their basenames → no hits).

All line ranges are "as of 17a9b5799"; item and test NAMES are authoritative if lines drift.

### Measured populations

| What | Count | Command |
|---|---|---|
| Spec citations of the ten files | 43 (20 helper_render, 6 canvas/mod.rs, 0 ×3 crypto, 1 gen_atomic_write, 0 gen_directory, 2 gen_read_write, 3 gen_math, 11 gen_fixed_math) | `grep -rnoF "[[src/codegen/builtins/<path>" src/docs/spec \| wc -l` per path |
| — of which already stale | 1 (`stdlib/07_math-rng.md:190 [[…/gen_math.rs:883]]`, a line number) | `grep -rn "gen_math.rs:883" src/docs/spec` |
| `.ai/` path mentions | 2 (`.ai/canvas-threading.md:842`, `:943`, both `helper_render.rs`) | `grep -rnE "helper_render\.rs\|gen_math\.rs\|gen_fixed_math\.rs\|func_generate\.rs\|gen_atomic_write\.rs" .ai` |
| Code/test comment mentions of the old paths | 32 (27 `gen_math.rs` / `gen_fixed_math.rs`, 3 `helper_render.rs`, 1 `gen_atomic_write.rs`, 1 `func_generate.rs`; listed in §4.11) | `grep -rnE "helper_render\.rs\|builtins/canvas/mod\.rs\|func_generate\.rs\|gen_atomic_write\.rs\|gen_math\.rs\|gen_fixed_math\.rs" src tests tools .ai --include='*.rs' --include='*.md' --include='*.py' \| grep -v '^src/docs/spec' \| wc -l` → 35 = these 32 + the 2 `.ai` lines + `audio/func_render.rs:4` (a different, non-existent `helper_render.rs`; §4.11) |
| Overflow labels in S files scanned by `no_overflow_label_returns_through_the_result_tag_register` | 3 (`gen_math.rs:900`, `gen_fixed_math.rs:1119`, `:1369`) | `grep -n 'abi::label(&.*overflow' <ten files>` |
| Non-test `#[cfg(...)]` gates in S files | 0 | `grep -n '#\[cfg(' <ten files>` → only the `#[cfg(test)]` of `canvas/mod.rs:2551`, `helper_render.rs:2424`, `gen_fixed_math.rs:1741` |
| Guard-allowlist / coverage-exception entries for S files | 0 | `grep -rn "<basename>" tests/guards scripts/coverage-exceptions.txt tools/vector-gen` → no hits |
| `include_str!`/`read_to_string` of S files | 0 | `grep -rnE 'include_str!\|read_to_string\|CARGO_MANIFEST_DIR' src tests --include='*.rs' \| grep -E 'builtins/(canvas\|crypto\|fs\|math\|money)'` → none naming an S file |

### Verified properties

- **The citation checker is file-level** (`src/docs/spec/mod.rs::spec_citations_resolve`,
  plan-176-A §2). Converting `foo.rs` → `foo/mod.rs` makes every `[[foo.rs:…]]` fail, which
  forces the sweep for nine of the ten files. `canvas/mod.rs` keeps its path, so its
  `JoinStyle` citation would pass silently after the symbol moves: §4.10 repoints it by hand.
- **Glob-cycle imports compile without warnings** (`use super::*;` in a part, `use part::*;` in
  `mod.rs`): probe `/tmp/loc/probe2` printed `6` with no warning. A child's `use super::*;` also
  brings the parent's private `use` imports into scope, so each part sees the original header
  imports and no `super::X` → `super::super::X` rewrite is needed (scratch crate
  `/tmp/loc/work_T/globtest/lib.rs`: an import only a child uses raised no `unused_imports`).
- **Cross-module `macro_rules!` raw-string pieces joined by `concat!` work** (probe
  `/tmp/loc/probe_concat`, edition 2021 like the repo's `Cargo.toml`): `p/a.rs` and `p/b.rs`
  each define `macro_rules! piece_x { () => { r#"…"# }; }` followed by `pub(super) use piece_x;`,
  and `p/mod.rs` has `const T: &str = concat!(a::piece_a!(), b::piece_b!());`.
  `cargo test` → `test result: ok. 1 passed` asserting `T == "x\ny\nz"` (the newline at the
  end of piece a is kept). The only warnings were the probe's own unused `T`/`t`.
- **The precedent exists in the same file:** `helper_render.rs:1794 macro_rules! render_loop_head`,
  `:1916 render_loop_tail`, `:2003 const RENDER_LOOP: &str = concat!(render_loop_head!(), render_loop_tail!());`.
- **Moving Rust code cannot change compiler output through source locations** (plan-176-A §2:
  no `module_path!`/`file!`/`line!` in `src`; no link-order registration). Registry entries carry
  no source location: `grep -rn "track_caller\|Location::caller" src/codegen/registry` → empty.
- **No host-`cfg` code in S.** Every platform emitter (macOS/Linux/Windows crypto, fs) is
  compiled on every host (`#[cfg(` count above), so `cargo check --all-targets` on the macOS host
  compiles all of S's code; no Windows or Linux `cargo check` is needed.
- **The scoped artifact gate reaches S's code** except canvas: `grep -rl "<pkg>::<member>"
  tests/byte-identity/*/src` finds `crypto::generate/sign/verify` (crypto),
  `fs::writeTextAtomic/listDirectory/createDirectories/readLine` (fs), `math::seed/rand/sin`
  (math), `money::` (money). `fs::seek` has no golden-owning caller (same grep over `tests/`
  → none); it is reached by `tests/runtime/rt_hand_built_record_validation.rs:122`. Canvas is
  reached only by `tests/syntax/app/app-mouse-surface` (`.ai/testing-gates.md`), which the
  per-phase check runs through `scripts/test-accept.sh` with that name glob.

## 3. Design Overview

**Conventions** (plan-176-A, `.ai/build-tooling.md` "Splitting an over-budget file"):
`<stem>.rs` → `<stem>/mod.rs` + parts (`git mv <stem>.rs <stem>/mod.rs` first so history follows).
The parent declaration (`mod <stem>;` / `pub(crate) mod <stem>;`) is unchanged. Each part starts
with `use super::*;` plus only the `use` lines the moved text needs that the parent does not
import; unused imports are trimmed per file (`unused_imports`). An item moved into a part that
`mod.rs` or a sibling uses becomes `pub(super)`; items left in `mod.rs` need no change
(descendants see private items). `CodeBuilder` methods move inside an `impl` block with the
original header in the part; a private method called from another part becomes `pub(super)`.
Free items that outside callers use are re-exported from `mod.rs` with `pub(crate) use`. ≤ 900
code LOC per part. Pure motion: text is moved verbatim (rustfmt may re-wrap; no token change).

**Gate class: byte-identity.** Every phase runs the common check sequence (Phases section):
`cargo build --release`, then the scoped `sh scripts/artifact-gate.sh target/release/mfb <builtin>`
for the built-ins whose lowering moved (named per phase, ~1 min each), the canvas fixture via
`test-accept.sh` for canvas phases, the moved modules' unit tests, `cargo check --all-targets`
with no new warning, `spec_citations_resolve`, the three name-based guards and the file-size
tighten. Letter end, once: `sh scripts/artifact-gate.sh target/release/mfb all` (15–20 min) —
code motion in codegen can change emission for any fixture on any target, and only the full gate
covers all targets and the canvas fixture. A diff is never a stop: localize it on ONE fixture
(`mfb build -q -ncode` with HEAD's compiler vs. the phase's, diff the two `.ncode`), fix the
motion or extraction, re-run.

**Risk concentrates** in Phase 4 (`RENDER_METAL_TEMPLATE`, where one dropped or doubled newline
at a cut changes the companion source of every canvas program, and the scoped gate cannot see
canvas) and Phase 5 (canvas `register`, where a reordered statement changes registry order). Phase
4 is guarded by an exact text-equality check of the joined literal (§4.1) plus the canvas fixture;
Phase 5 by verbatim contiguous runs called in source order plus the canvas fixture and the canvas
unit tests. Phase 5 is the only extract-method phase and is last.

**Overflow-label corpus.** Phase 3 moves three scanned overflow labels into parts whose file
names do not start with `builder_`/`func_`/`gen_`: `gen_math/scalar.rs`, `gen_fixed_math/trig.rs`
and `gen_fixed_math/exp_log.rs`. plan-176-O Phase 1 (a prerequisite through letter order) makes
`collect_builder_sources` also collect every `.rs` file below a `builder_*`/`func_*`/`gen_*`
directory, so the moved labels stay in the corpus. S edits no guard; Phase 3's Acceptance proves
the three files are scanned.

**Rejected alternatives.**
- *Exempt canvas `register` (2093) or `RENDER_METAL_TEMPLATE` (1464)* — the owner decided single
  functions over the limit are decomposed by extract-method (plan-176-A §Decisions), and the
  author decided the template uses the in-file `concat!` precedent (brief). The S map's
  exemption notes are superseded by §4.1 and §4.10; since 2026-10-18 the rule has no exemptions at all.
- *Split `RENDER_METAL_TEMPLATE` into separately registered helpers* — changes helper names and
  registration order, so the companion source changes. `concat!` keeps the text identical.
- *Cut the template at other lines* — the three cuts (after 419, after 1035) are MFBASIC
  top-level boundaries (a blank line after an `END FUNC`), so every piece is a whole set of
  functions with its own comments.
- *A struct for canvas `register`'s state* — the only local is `pkg`; each sub-registrar takes
  `&mut RegistryPackage`, the existing `func_fill::register` signature.
- *Name the canvas sub-registrar files `func_types_*`/`helper_types_*`* — the `func_*` prefix is
  what `no_cross_member_os_seam_dispatcher` keys on and what readers expect to hold a member;
  `types_*` says what the files hold.
- *Re-pointing `bugs/*.md`* — bug documents record the tree of their time.

## 4. Detailed Design

LOC figures: `python3 -I /tmp/loc/span.py <file> <a> <b>` over the stated ranges (sums over
several ranges via `/tmp/loc/rs_work/spans.sh`). "+N" = the part's own `use`/`impl`/`mod` lines
(guess). Re-measure every new file with `python3 -I tools/loc-census/census.py --file <path>`
before committing.

### 4.1 `canvas/helper_render.rs` → `canvas/helper_render/` (Phase 4)

`RENDER_METAL_TEMPLATE` is one raw string: line 149 `#[rustfmt::skip]`, line 150
`const RENDER_METAL_TEMPLATE: &str =`, text lines 151-1612 (line 151 starts `r#"' plan-116-G`,
line 1612 is `END FUNC"#;`), 1464 LOC.

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `helper_render/mod.rs` | 1-20, 97-150, 1613-1655, 2387-2425 | ≈100 | `//!` docs and `use`s (incl. `runtime::canvas::{…caps}`, `LazyLock`); the `RENDER_METAL_TEMPLATE` doc and `#[rustfmt::skip]`, with the const now `const RENDER_METAL_TEMPLATE: &str = concat!(gpu_groups::render_metal_groups!(), gpu_draw_list::render_metal_draw_list!(), gpu_passes::render_metal_passes!());`; `RENDER_CAPS` and the static `RENDER_METAL` (1613-1655); `register` and `#[cfg(test)] mod tests;` (2387-2425); `mod scene; mod gpu_groups; mod gpu_draw_list; mod gpu_passes; mod render_loop; mod present;`; `use scene::{HASH_SCENE, RENDER_SCENE}; use render_loop::{ENSURE_GRAPHICS, TEST_SWITCHES, TEST_SWITCHES_DEBUG, RENDER_LOOP, PHASE_TIMERS, PHASE_TIMERS_DEBUG, POINTER_SCRIPT, POINTER_SCRIPT_DEBUG, RENDER_LOOP_DEBUG}; use present::{DIRECT_PRESENT, DIRECT_PRESENT_DEBUG, CLOSE_RETIRED};` |
| `helper_render/scene.rs` | 21-96, 1656-1697 | 97 | `RENDER_SCENE`, `HASH_SCENE` |
| `helper_render/gpu_groups.rs` | 151-419 | ≈272 (269+3) | `macro_rules! render_metal_groups { () => { r#"<line 151 after its r#" … line 418 END FUNC, the blank line 419, ending in a newline>"# }; }` + `pub(super) use render_metal_groups;` — group composition: `__canvas_groupRow`, `__canvas_composeGroup`, `__canvas_appendDraw`, `__canvas_noteEntry`, `__canvas_groupSignature`, `__canvas_intListEquals` |
| `helper_render/gpu_draw_list.rs` | 420-1035 | ≈619 (616+3) | `render_metal_draw_list`, from 420 (`' plan-116-H: the GPU draw list…`) through the blank 1035 after `__canvas_sceneOffsets`'s `END FUNC`: the draw list, memo, `__canvas_pushOneDraw`, `__canvas_sceneDraws*`, `__canvas_sceneOffsets` |
| `helper_render/gpu_passes.rs` | 1036-1612 | ≈580 (577+3) | `render_metal_passes`, from 1036 (`' The frame buffer's item region…`) through the final `END FUNC` of 1612 (the `"#;` becomes the macro's `"#`): the `@CAP@` LETs, `__canvas_blockCost`, `__canvas_framePasses`, `__canvas_metalRenderable`, `__canvas_vulkanRenderable`, `__canvas_passNext`, `__canvas_renderMetal`, `__canvas_renderVulkan` |
| `helper_render/render_loop.rs` | 1698-2188 | 385 | `ENSURE_GRAPHICS`, `TEST_SWITCHES`, `TEST_SWITCHES_DEBUG`, `macro_rules! render_loop_head`, `render_loop_tail`, `RENDER_LOOP`, `PHASE_TIMERS`, `PHASE_TIMERS_DEBUG`, `POINTER_SCRIPT`, `POINTER_SCRIPT_DEBUG`, `RENDER_LOOP_DEBUG` (the macros stay above their two `concat!` users in this file) |
| `helper_render/present.rs` | 2189-2386 | 157 | `DIRECT_PRESENT`, `DIRECT_PRESENT_DEBUG`, `CLOSE_RETIRED` |

Surface/visibility: 14 consts → `pub(super)` (`RENDER_SCENE`, `HASH_SCENE`, `ENSURE_GRAPHICS`,
`TEST_SWITCHES`, `TEST_SWITCHES_DEBUG`, `RENDER_LOOP`, `PHASE_TIMERS`, `PHASE_TIMERS_DEBUG`,
`POINTER_SCRIPT`, `POINTER_SCRIPT_DEBUG`, `RENDER_LOOP_DEBUG`, `DIRECT_PRESENT`,
`DIRECT_PRESENT_DEBUG`, `CLOSE_RETIRED`); 3 macro `pub(super) use` re-exports. The test module
(`helper_render/tests/mod.rs`, reads `RENDER_SCENE, RENDER_METAL_TEMPLATE, RENDER_CAPS, RENDER_METAL,
RENDER_LOOP, RENDER_LOOP_DEBUG` through `use super::*`) sees the moved consts through mod.rs's
private `use` lines. The raw-string parts need no `use super::*;` (they reference nothing).

Text-identity check (the strongest check for this file; run before the scoped gate):
```
git show HEAD:src/codegen/builtins/canvas/helper_render.rs > /tmp/hr_old.rs   # before editing
python3 -I -c "import re
old='\n'.join(open('/tmp/hr_old.rs').read().split('\n')[150:1612]); old=old[3:-3]
d='src/codegen/builtins/canvas/helper_render/'
new=''.join(re.search(r'r#\"(.*?)\"#', open(d+p+'.rs').read(), re.S).group(1) for p in ('gpu_groups','gpu_draw_list','gpu_passes'))
print(old==new, len(old), len(new))"
```
→ `True` and two equal lengths. (`old[3:-3]` strips line 151's leading `r#"` and line 1612's
trailing `"#;`; a raw string cannot contain `"#`, so the non-greedy match takes each whole body.)

Citations (20), all repointed in Phase 4: `app/06_canvas.md` — `ENSURE_GRAPHICS` (:334),
`render_loop_head` (:351), `RENDER_LOOP` (:1202), `POINTER_SCRIPT_DEBUG` (:2182) →
`helper_render/render_loop.rs`; `RENDER_METAL` (:419), `RENDER_METAL_TEMPLATE` (:1405, :1498) →
`helper_render/mod.rs`; `__canvas_blockCost` (:420), `__canvas_framePasses` (:421),
`__canvas_passNext` (:422), `__canvas_metalRenderable` (:442), `__canvas_vulkanRenderable`
(:443), `__canvas_renderMetal` (:396, :468), `__canvas_renderVulkan` (:469) →
`helper_render/gpu_passes.rs`; `__canvas_pushOneDraw` (:554) → `helper_render/gpu_draw_list.rs`.
`tooling/09_debug-report.md` — `ENSURE_GRAPHICS` (:157), `TEST_SWITCHES_DEBUG` (:208),
`PHASE_TIMERS_DEBUG` (:226) → `helper_render/render_loop.rs`; `DIRECT_PRESENT_DEBUG` (:284) →
`helper_render/present.rs`. (Line numbers as of 17a9b5799; locate by
`grep -rnoE "\[\[src/codegen/builtins/canvas/helper_render\.rs:[^]]+\]\]" src/docs/spec`.)

### 4.2 `crypto/func_generate.rs` → `crypto/func_generate/` (Phase 2)

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `func_generate/mod.rs` | 1-102, 129-130, 1647-2085 | ≈474 | docs, `use`, `OsslCurve`, `OSSL_CURVES`, `WinCurve`, `WIN_CURVES`; `LOCAL_SIZE`; `lower_generate`, `INTRO`, `DESC`, `EX`, `register`; `mod macos; mod linux; mod windows;` |
| `func_generate/macos.rs` | 103-128, 131-545 | ≈409 | macOS frame consts `SEC`…`RBLOCK`, `emit_macos_ec` |
| `func_generate/linux.rs` | 546-1151 | ≈559 | `L_*` frame consts, `emit_linux_ec` |
| `func_generate/windows.rs` | 1152-1646 | ≈461 | `W_*` consts, `win_cleanup`, `emit_windows_ec` |

Surface/visibility: 12 → `pub(super)`: `emit_macos_ec`, `emit_linux_ec`, `emit_windows_ec`;
`L_NID`, `L_POINTLEN`, `L_FIELDLEN`, `L_SEC1OFF`, `L_SPKIPREFIX`, `L_NAMEPTR`; `W_FIELD`,
`W_ALGOPTR`, `W_BITS` (`lower_generate` writes these frame slots). `OSSL_CURVES`/`WIN_CURVES`
stay in `mod.rs`. The module stays `pub(crate) mod func_generate`; no outside importer
(`grep -rn "func_generate::" src` → none). Citations 0. Comment repoint:
`tests/runtime/rt_debug_soak.rs:707` (`crypto/func_generate.rs` → `crypto/func_generate/mod.rs`,
which holds `lower_generate`'s `fromString` path).

### 4.3 `crypto/func_sign.rs` → `crypto/func_sign/` (Phase 2)

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `func_sign/mod.rs` | 1-142, 1440-1808 | ≈449 | `use`, `SignCurve` + impl, `LOCAL_SIZE`, `lower_sign`, `INTRO`, `DESC`, `EX`, `register`; `mod macos; mod linux; mod windows;` |
| `func_sign/macos.rs` | 143-428 | ≈263 | `emit_macos_sign` |
| `func_sign/linux.rs` | 429-912 | ≈445 | `lx_free_guarded`, `emit_linux_sign` |
| `func_sign/windows.rs` | 913-1439 | ≈484 | `win_der_encode_int`, `win_import_private_key`, `win_hash_message`, `win_cleanup`, `emit_windows_sign` |

Surface/visibility: 3 → `pub(super)` (`emit_macos_sign`, `emit_linux_sign`, `emit_windows_sign`).
`SignCurve` and its private methods stay in `mod.rs`. The intra-doc link
``[`super::func_sign::lower_sign`]`` in `func_verify`'s `//!` still resolves (`lower_sign` stays in
`func_sign/mod.rs`; `super` from `func_verify/mod.rs` is still `crypto`). Citations 0; path refs 0.

### 4.4 `crypto/func_verify.rs` → `crypto/func_verify/` (Phase 2)

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `func_verify/mod.rs` | 1-120, 1379-1684 | ≈362 | `use`, `VerifyCurve` + impl, `LOCAL_SIZE`, `lower_verify`, `INTRO`, `DESC`, `EX`, `register`; 3 `mod` |
| `func_verify/macos.rs` | 121-392 | ≈244 | `emit_macos_verify` |
| `func_verify/linux.rs` | 393-804 | ≈374 | `lx_free_guarded`, `emit_linux_verify` |
| `func_verify/windows.rs` | 805-1378 | ≈513 | `win_der_decode_int`, `win_import_public_key`, `win_hash_message`, `win_cleanup`, `emit_windows_verify` |

Surface/visibility: 3 → `pub(super)` (`emit_macos_verify`, `emit_linux_verify`, `emit_windows_verify`).
Citations 0; path refs 0.

### 4.5 `fs/gen_atomic_write.rs` → `fs/gen_atomic_write/` (Phase 1)

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `gen_atomic_write/mod.rs` | 1-49, 559-813 | ≈260 | `use`, `normalize_c_int_result`, `AtomicWriteValueKind`, `lower_fs_write_path_helper`; `mod atomic; mod read_path;` `pub(crate) use atomic::lower_fs_atomic_write_helper;` `pub(crate) use read_path::{lower_fs_read_bytes_path_helper, lower_fs_read_text_path_helper};` |
| `gen_atomic_write/atomic.rs` | 50-558 | ≈457 | `lower_fs_atomic_write_helper` |
| `gen_atomic_write/read_path.rs` | 814-1359 | ≈486 | `lower_fs_read_text_path_helper`, `lower_fs_read_bytes_path_helper` |

Surface/visibility: 2 re-export lines (3 names); the moved fns keep `pub(crate)`. No visibility
edits: `normalize_c_int_result` and `AtomicWriteValueKind` stay in `mod.rs`. Callers
(`fs/func_write_bytes(_atomic)`, `func_write_text(_atomic)`, `func_append_*`, `func_read_text`,
`func_read_bytes` through `super::gen_atomic_write::…`, incl. `AtomicWriteValueKind::Bytes/String`)
are unchanged. Citation 1: `stdlib/21_fs-host-errors.md:60 lower_fs_read_text_path_helper` →
`fs/gen_atomic_write/read_path.rs`. Comment repoint: the `proof` string naming
`builtins/fs/gen_atomic_write.rs` in the `SELF_UPDATE_TABLE` row for `lower_fs_read_text_path_helper`
(`src/codegen/collection/assign/self_update.rs:2313` at planning; plan-176-J moves the table to
its own file — locate with `grep -rn "builtins/fs/gen_atomic_write.rs" src`) →
`builtins/fs/gen_atomic_write/read_path.rs`. The table's own test only requires a non-empty
`proof` (`self_update.rs:3420 !proof.trim().is_empty()`), so the edit changes no test outcome.

### 4.6 `fs/gen_directory.rs` → `fs/gen_directory/` (Phase 1)

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `gen_directory/mod.rs` | 1-402 | ≈361 | `use`, `lower_fs_current_directory_helper`, `lower_fs_temp_directory_helper`, `lower_fs_path_operation_helper`; `mod create; mod list;` `pub(crate) use create::lower_fs_create_directories_helper; pub(crate) use list::lower_fs_list_directory_helper;` |
| `gen_directory/create.rs` | 404-760 | ≈321 | `lower_fs_create_directories_helper`, `emit_windows_create_directories_start` |
| `gen_directory/list.rs` | 762-1151 | ≈352 | `lower_fs_list_directory_helper` |

Surface/visibility: 2 re-exports, 0 visibility edits (no cross-part use: `/tmp/loc/rs_work/xref.py`).
Citations 0; path refs 0.

### 4.7 `fs/gen_read_write.rs` → `fs/gen_read_write/` (Phase 1)

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `gen_read_write/mod.rs` | 1-710 | ≈682 | `use`, `lower_fs_write_all_helper`, `lower_fs_read_all_helper`, `lower_fs_write_all_bytes_helper`, `lower_fs_read_all_bytes_helper`; `mod position; mod line;` `pub(crate) use position::{lower_fs_eof_helper, lower_fs_read_bytes_at_helper, lower_fs_seek_helper, lower_fs_size_helper};` `pub(crate) use line::lower_fs_read_line_helper;` |
| `gen_read_write/position.rs` | 712-1344 | ≈553 | `lower_fs_eof_helper`, `lower_fs_size_helper`, `lower_fs_seek_helper`, `lower_fs_read_bytes_at_helper` |
| `gen_read_write/line.rs` | 1346-1864 | ≈430 | `emit_append_to_line_accumulator`, `emit_reconcile_read_buffer`, `lower_fs_read_line_helper` |

Surface/visibility: 2 re-export lines (5 names). One visibility edit: `emit_reconcile_read_buffer`
(`line.rs`) is used by the whole-file helpers in `mod.rs` → `pub(super)`. Callers in `fs/func_*.rs`
are unchanged. Citations 2: `stdlib/23_fs-memory-files.md:84 lower_fs_seek_helper` →
`fs/gen_read_write/position.rs`; `:91 lower_fs_read_line_helper` → `fs/gen_read_write/line.rs`.

### 4.8 `math/gen_math.rs` → `math/gen_math/` (Phase 3)

The single `impl CodeBuilder` (lines 15-1352, 1095 LOC) splits into one `impl` block per file
(legal for an inherent impl; each part repeats the original `impl` header).

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `gen_math/mod.rs` | 1-89, 1199-1412 | ≈200 | `use`, `FloatInfinityError`, `impl CodeBuilder { lower_math_call, is_list_argument, emit_float_result_check, emit_float_result_check_fp, observe_float, observe_promoted_float }`, `float_arith_node`; `mod arrays; mod scalar; mod rng;` |
| `gen_math/arrays.rs` | 90-176, 312-455, 505-609 | ≈286 | `impl CodeBuilder { numeric_element_type_code, list_element_type, lower_math_abs_array, lower_math_sqrt_array, lower_math_atan2_pow_array, lower_math_exp_array, lower_math_trig_array, lower_math_log_array, lower_math_rounding_array, lower_math_min_max_array, lower_math_clamp_array }` |
| `gen_math/scalar.rs` | 178-310, 457-503, 611-904, 1100-1197 | ≈508 | `impl CodeBuilder { lower_math_scalar_transcendental, lower_math_scalar_binary, lower_math_abs, lower_math_min_max, lower_math_clamp, lower_math_rounding, emit_float_exponent_classify, emit_float_rounding_integer_range_check, lower_math_sqrt, lower_fixed_external_math }` |
| `gen_math/rng.rs` | 906-1098 | ≈154 | `impl CodeBuilder { emit_rng_next_call, lower_math_rand, lower_math_seed }` |

Methods keep their source order within each part (the ranges are ascending). Visibility: the 18
methods called from `lower_math_call` (`grep -n "\.<m>(" gen_math.rs`) → `pub(super)`: 9 in
arrays (every `lower_math_*_array`), 7 in scalar (`lower_math_scalar_transcendental`,
`lower_math_scalar_binary`, `lower_math_abs`, `lower_math_min_max`, `lower_math_clamp`,
`lower_math_rounding`, `lower_math_sqrt`), 2 in rng (`lower_math_rand`, `lower_math_seed`).
`lower_fixed_external_math`, `emit_float_rounding_integer_range_check`, `emit_rng_next_call`,
`numeric_element_type_code` and `list_element_type` are called only inside their own part and stay
private; `emit_float_exponent_classify` is already `pub(crate)`. `math/mod.rs:80 pub(crate) use gen_math::*;`
still re-exports `FloatInfinityError`/`float_arith_node`; outside users
(`collections/func_sum.rs` `math::gen_math::FloatInfinityError`, `float_result.rs`'s doc link
`gen_math::float_arith_node`) are unchanged. The overflow label at line 900
(`emit_float_rounding_integer_range_check`) moves to `scalar.rs`.

Citations 3: `architecture/18_math-kernels.md:12 lower_math_call` → `math/gen_math/mod.rs`;
`stdlib/07_math-rng.md:180 lower_math_rand` → `math/gen_math/rng.rs`;
`stdlib/07_math-rng.md:190 [[src/codegen/builtins/math/gen_math.rs:883]]` → **pre-existing stale
line citation**: line 883 is inside `emit_float_rounding_integer_range_check`, the paragraph is
about `math::seed`, and `bugs/bug-873-stdlib-05-10-spec-drift.md` item 2 records the intended
target `lower_math_seed` (`grep -n "fn lower_math_seed" src/codegen/builtins/math/gen_math.rs` →
1073). Repoint to `[[src/codegen/builtins/math/gen_math/rng.rs:lower_math_seed]]`.

### 4.9 `money/gen_fixed_math.rs` → `money/gen_fixed_math/` (Phase 3)

The single `impl CodeBuilder` (lines 87-1719, 1292 LOC) splits into three blocks.

| New file | Source lines | LOC | Contents |
|---|---|---|---|
| `gen_fixed_math/mod.rs` | 1-380, 1719-1812 | ≈266 code + 60 test | docs, `use`, every const (`FIXED_*`, `Q63_ONE`, `TWO_OVER_PI_Q64`, `PI_*`, `FIXED_ATAN_TERMS`), `pi_over_2_scaled`, `ReducedAngle`, `impl CodeBuilder { emit_fixed_rounding_to_integer, emit_fixed_sqrt, emit_const_i64, emit_const_u64, emit_fixed_mul, emit_fixed_mul_inplace, emit_fixed_mul_saturating }`, `fixed_raw`, `fixed_ln2`, `fixed_inv_ln2`, `fixed_inv_ln10`, the inline `#[cfg(test)] mod tests` (unchanged; it imports `super::{pi_over_2_scaled, FIXED_ATAN_TERMS, PI_FIXED, PI_OVER_2_FIXED, PI_OVER_2_Q159, PI_OVER_2_Q61, PI_OVER_4_Q61, PI_Q61, TWO_OVER_PI_Q64}`, all staying here); `mod trig; mod exp_log;` |
| `gen_fixed_math/trig.rs` | 381-1223 | ≈669 | `impl CodeBuilder { emit_fixed_atan_ratio, emit_q61_to_signed_fixed, emit_fixed_atan2, emit_fixed_trig_reduce, emit_fixed_sin_cos_magnitudes, emit_q63_to_signed_fixed, emit_conditional_negate, emit_fixed_sin_cos, emit_fixed_tan, emit_fixed_asin }` |
| `gen_fixed_math/exp_log.rs` | 1224-1718 | ≈409 | `impl CodeBuilder { emit_fixed_exp, emit_fixed_scale_by_power_of_two, emit_fixed_log, emit_fixed_pow_general }` |

Visibility: **0 edits**. Every private method is called only inside its own part (call sites:
atan_ratio 633/1200, q61 642/1220, trig_reduce 863/935, sin_cos_magnitudes 864/1057, q63 905,
conditional_negate 570/833/1116, scale_by_power_of_two 1327, log 1686, exp 1702). The `pub(crate)`
methods stay `pub(crate)`. Overflow labels at 1119 (trig) and 1369 (exp_log) move with their fns.

Citations 11 in `architecture/18_math-kernels.md`: `emit_fixed_trig_reduce` (:81),
`emit_fixed_sin_cos_magnitudes` (:82), `emit_fixed_tan` (:90), `emit_fixed_atan_ratio` (:100),
`emit_q61_to_signed_fixed` (:110), `emit_fixed_atan2` (:116), `emit_fixed_asin` (:128) →
`money/gen_fixed_math/trig.rs`; `emit_fixed_exp` (:135), `emit_fixed_scale_by_power_of_two`
(:136), `emit_fixed_pow_general` (:158) → `money/gen_fixed_math/exp_log.rs`;
`emit_fixed_mul_saturating` (:157) → `money/gen_fixed_math/mod.rs`.

### 4.10 Extract-method: canvas `register` (Phase 5)

Function: `src/codegen/builtins/canvas/mod.rs:247-2508`, `pub(crate) fn register(r: &mut Registry)`,
2093 code LOC (`bash /tmp/loc/spans.sh src/codegen/builtins/canvas/mod.rs 247-2508`). The S-extract
map's corrected ranges are used: `add_imports` ends at **278**, and `types_values` is **280-741**.

**Structure** (top-level statements: `awk 'NR>=247&&NR<=2510 && /^    [^ \/]/'`):

| id | lines | LOC | content | → |
|---|---|---|---|---|
| H | 247-278 | 14 | signature; `let mut pkg = RegistryPackage::new("canvas", MODULE_INTRO, MODULE_DESC);`; comment; `pkg.add_imports(vec![…]);` | stays |
| A | 280-741 | 436 | `// ---- Value types…` banner (280), plan-122-D comment, 14 statements: `MouseEvent` (`MOUSE_EVENT_TYPE`), `MouseKind`, `MouseButton`, `Size`, `TextMetrics`, `BlendMode`, `GradientKind`, `GradientSpread`, `GradientApply`, `Renderer`, `FrameStats`, `CapStyle`, `JoinStyle`, `FillRule` | `canvas/types_values.rs` |
| B | 743-1079 | 318 | `Transform`, `GradientStop`, `Gradient`, `Paint` | `canvas/types_paint.rs` |
| C | 1081-1620 | 501 | `// ---- The DrawItem variants (a CLOSED set)` banner (1081), `BindTo`, `Rectangle`, `Line`, `Polygon`, `Circle`, `Arc`, `Text`, `PictureClip`, `PictureFit`, `Tile`, `Picture`, unions `Shape` (1533), `AutoShape` (1581), `DrawItem` (1591) | `canvas/types_items.rs` |
| D | 1622-2075 | 441 | plan-150-A comment (1622-1627), `ParticleSystem`, `EmissionMode`, `Emission`, `Motion`, `Appearance`, `AutoPaint` | `canvas/types_motion.rs` |
| E | 2077-2378 | 289 | `Group`, `Ellipse`, `PolyLine`, `BezierCurve`, `PolyCurve`, `CurvePolygon`, `DrawLayer` | `canvas/types_groups_curves.rs` |
| T | 2380-2508 | 94 | `// ---- Resources` banner and comment; `add_resource` ×2 (Image 2392, Font 2416); the 64 `func_*/gen_*/helper_*::register(&mut pkg)` calls (2439-2505); `r.add_package(pkg)` | stays |

Sum 14+436+318+501+441+289+94 = 2093. Blank separators 279, 742, 1080, 1621, 2076, 2379 carry
no LOC. A–E hold 45 statements (14 + 4 + 14 + 6 + 7), every one `pkg.add_record|add_enum|add_union(…);`.

**Control-flow audit.** `grep -nE "\breturn\b|\?;|\blet\b|\bfor\b|\bif\b|\|[a-z_]*\||\bmatch\b|\bloop\b"`
over 247-2509 hits only inside string literals; the one `let` is `pkg` at 248. No early exit, no
`?`, no closure, no loop, no branch. Every A–E statement is a unit expression statement
(`add_*` returns `&mut Self`, discarded by `;`), so each run becomes a plain fall-through call.

**Shared state.** `pkg: RegistryPackage` is the only local. Each sub-registrar takes
`pkg: &mut RegistryPackage` (the signature of `func_fill::register`, `func_fill.rs:80`); the bodies
are verbatim with zero token edits (`pkg.add_record(…)` autoreborrows). Names the moved statements
read, all reachable through `use super::*;`: `MOUSE_EVENT_TYPE` (A), `paint_prop()`/`id_prop()`/
`transform_prop()` (C: 6/7/6, E: 4/5/4; private fns of `canvas/mod.rs`, visible to children),
`ParameterType`, `RegistryRecord`, `RegistryEnum`, `RegistryUnion`, `RecordProp`, `EnumVariant`,
`UnionVariant` (mod.rs's `use crate::codegen::registry::{…}`/`use crate::types::ParameterType`),
and absolute `crate::codegen::builtins::{vector,color}::…` paths. No visibility edits beyond the
five new fns. Each child's own `fn register` shadows the glob-imported `canvas::register` (an
explicit item beats a glob).

**Each new file:**
```rust
use super::*;

/// <section>: registered by `canvas::register`, in this position (registration order is
/// render order).
pub(super) fn register(pkg: &mut RegistryPackage) {
    <lines a-b verbatim, already at 4-space indentation>
}
```
| file | lines moved | LOC |
|---|---|---|
| `canvas/types_values.rs` | 280-741 | ≈439 |
| `canvas/types_paint.rs` | 743-1079 | ≈321 |
| `canvas/types_items.rs` | 1081-1620 | ≈504 |
| `canvas/types_motion.rs` | 1622-2075 | ≈444 |
| `canvas/types_groups_curves.rs` | 2077-2378 | ≈292 |

(+3 per file is the `use`, `fn` line and `}`, guess.)

**Remainder and emission order:**
```rust
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("canvas", MODULE_INTRO, MODULE_DESC);   // H verbatim
    /* comment */ pkg.add_imports(vec![ … ]);
    types_values::register(&mut pkg);          // A  280-741
    types_paint::register(&mut pkg);           // B  743-1079
    types_items::register(&mut pkg);           // C  1081-1620
    types_motion::register(&mut pkg);          // D  1622-2075
    types_groups_curves::register(&mut pkg);   // E  2077-2378
    /* T verbatim: Resources banner, add_resource ×2, 64 sub-registrar calls, r.add_package(pkg) */
}
```
The 45 `add_*` calls run in exactly the original order; records, unions and enums render into
`get_mfb` in the order they are added (`src/codegen/registry` `add_record`/`add_union`/`add_enum`
docs), so the companion source and every downstream id and order are identical. The resources
stay after the types (`Picture`/`Text` name `RES canvas::Image`/`Font` by string). Remaining
`register` ≈113 LOC (guess: 14 + 5 + 94). `canvas/mod.rs` gains
`mod types_values; mod types_paint; mod types_items; mod types_motion; mod types_groups_curves;`
(private, like the 73 existing `mod` decls). `canvas/mod.rs` afterwards ≈330 code LOC (guess:
2307 − 1985 + 10).

**Doc comment.** In `register`'s doc, "The type set is declared here as registry data" becomes
"The type set is declared as registry data by the `types_*` sub-registrars this calls, in order".
The paragraph on the frozen `DrawItem` set stays on `register` (it names the test
`draw_item_variant_set_is_frozen`, prose only).

**Citations.** `app/06_canvas.md:664 [[src/codegen/builtins/canvas/mod.rs:JoinStyle]]` →
`[[src/codegen/builtins/canvas/types_values.rs:JoinStyle]]` (the file-level checker would pass the
old path, so this is repointed by hand). `:32 CANVAS` (anchor comment at mod.rs:2548) and
`language/18_builtin-functions.md:110 register` stay. The three test citations are plan-176-H's.
The prose mention of `vulkan.rs` at `canvas/mod.rs:541` (listed in the V map) moves into
`types_values.rs` with region A; plan-176-V finds it by `grep -rn "vulkan.rs" src/codegen/builtins/canvas`.

**Hazards.** Borrowck: none (`&mut pkg` sequential; moved only at `r.add_package(pkg)`). Guards:
`no_type_strings` buckets by the first directory under `src/` (`codegen` for old and new files);
`no_cross_member_os_seam_dispatcher` keys on `func_*` names and `_os_seam` text, which `types_*`
files have neither of; no test reads `canvas/mod.rs` by path. Move each range **whole**, with its
leading banner and comments (280, 1081, 1622-1627); do not reorder the five calls; do not
interleave them with `add_resource`.

### 4.11 `.ai`, code-comment and stale-reference sweep

Each item is done in the phase that moves its target (phase in brackets).

| Site | Old | New | Phase |
|---|---|---|---|
| `.ai/canvas-threading.md:842` | `` `helper_render.rs` `` (`__canvas_metalRenderable`/`__canvas_vulkanRenderable`) | `` `helper_render/gpu_passes.rs` `` | 4 |
| `.ai/canvas-threading.md:943` | `` `TEST_SWITCHES_DEBUG` in `helper_render.rs` `` | `` … in `helper_render/render_loop.rs` `` | 4 |
| `src/codegen/runtime/canvas/mod.rs:1348` | `(`helper_render.rs`)` (the generated MFBASIC caps copy) | `(`helper_render/mod.rs`, `RENDER_CAPS`)` | 4 |
| `src/codegen/builtins/canvas/func_pointer.rs:10` | `helper_render.rs:__canvas_pointerScript` | `helper_render/render_loop.rs:__canvas_pointerScript` | 4 |
| `tests/canvas/rt_canvas_pointer.rs:10` | `helper_render.rs:POINTER_SCRIPT_DEBUG` | `helper_render/render_loop.rs:POINTER_SCRIPT_DEBUG` | 4 |
| `src/codegen/builtins/audio/func_render.rs:4` | `(`helper_render.rs`)` — **pre-existing stale**: there is no `audio/helper_render.rs` (`ls` → no such file); the body is `func_render.rs`'s own `BODY` (`grep -n "FUNC __audio_render" src/codegen/builtins/audio/func_render.rs` → 22) | `(`BODY` below)` | 4 |
| `tests/runtime/rt_debug_soak.rs:707` | `crypto/func_generate.rs` | `crypto/func_generate/mod.rs` | 2 |
| the `SELF_UPDATE_TABLE` proof string (§4.5) | `builtins/fs/gen_atomic_write.rs` | `builtins/fs/gen_atomic_write/read_path.rs` | 1 |
| `src/codegen/builtins/math/func_*.rs` (21 lines: `func_abs:48`, `func_log:47`, `func_clamp:73`, `func_max:48`, `func_seed:45`, `func_acos:45`, `func_cos:47`, `func_pow:58`, `func_atan:41`, `func_rand:62`, `func_sin:47`, `func_tan:55`, `func_round:45`, `func_atan2:48`, `func_log10:47`, `func_exp:47`, `func_floor:44`, `func_asin:44`, `func_ceil:43`, `func_min:48`, `func_sqrt:46`) | "`lower_math_call` carrier in `gen_math.rs`" | "… in `gen_math/mod.rs`" | 3 |
| `src/codegen/builtins/vector/builder_simd_float_math.rs:2535` | `(`gen_math.rs`)` | `(`gen_math/`)` (names the module; plan-176-T then moves this line with its method) | 3 |
| `src/codegen/builtins/math/func_cos.rs:37`, `func_sin.rs:37`, `func_atan.rs:31` | `` `money/gen_fixed_math.rs` `` | `` `money/gen_fixed_math/` `` (the comment names the module's error classes) | 3 |
| `src/codegen/builtins/tests/declared_errors_all.rs:951` (a 797-LOC test-only file, ≤ 1000, so no letter moves it) | `` `emit_fixed_log` (`money/gen_fixed_math.rs`) `` | `` … (`money/gen_fixed_math/exp_log.rs`) `` | 3 |
| `src/codegen/string/format/float_parse_table.rs:10` | `src/codegen/builtins/money/gen_fixed_math.rs` | `src/codegen/builtins/money/gen_fixed_math/` | 3 |

Each phase ends with a sweep command proving no live mention remains:
`grep -rnE "<old basenames of the phase>" src tests tools .ai --include='*.rs' --include='*.md' --include='*.py' | grep -v '^src/docs/spec' | grep -v 'audio/func_render'`
→ no hit naming an old path (bug and planning documents are not in the searched set).

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

**Common per-phase check sequence** (each phase's Acceptance names which built-ins and filters):

1. Before editing: `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` → record N;
   `cargo check --all-targets 2>&1 | grep -c '^warning'` → record W.
2. After editing: `rustfmt --edition 2021 <each new/changed .rs file>`; `git diff --stat` shows only this phase's files.
3. `cargo check --all-targets 2>&1 | grep -c '^warning'` → W (no new warning) (est. 2 min, guess).
4. `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$'` → N (est. 2 min, guess).
5. `cargo test -p mfb --bin mfb spec_citations_resolve` → passes (est. 2 min, guess).
6. `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings` → pass (est. 2 min, guess).
7. `cargo build --release` (est. 5 min, guess), then each named
   `sh scripts/artifact-gate.sh target/release/mfb <builtin>` → 0 diffs (~1 min each). Do not run
   two gates at once (`pgrep -f "artifact-gate|test-accept"` empty first; `.ai/testing-gates.md`).
8. `cargo test --test file_size_budget 2>&1 | grep -E 'src/codegen/builtins/(canvas|crypto|fs|math|money)/'`
   → only the rows the phase names (a `STALE` row for each converted `foo.rs`, whose path is gone);
   then `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget` and commit
   `tests/guards/file_size_ceilings.txt` with the phase (est. 1 min, guess).
9. Every new file: `python3 -I tools/loc-census/census.py --file <path>` → code ≤ 900, inline test ≤ 250.
10. The phase's sweep command (§4.11) → no hit naming an old path of the phase.

### Phase 1 — fs helper carriers (pure motion)

Three files with no visibility surprises and three citations; lowest risk.

- [x] `fs/gen_atomic_write.rs` → `fs/gen_atomic_write/{mod,atomic,read_path}.rs` per §4.5
      (`git mv` to `gen_atomic_write/mod.rs` first); the 2 re-export lines.
- [x] `fs/gen_directory.rs` → `fs/gen_directory/{mod,create,list}.rs` per §4.6; the 2 re-exports.
- [x] `fs/gen_read_write.rs` → `fs/gen_read_write/{mod,position,line}.rs` per §4.7; the 2
      re-export lines; `emit_reconcile_read_buffer` → `pub(super)`.
      Done (all three): `codesplit.py` per §4.5–4.7; the only visibility edit is `emit_reconcile_read_buffer`, which
      `mod.rs` reaches through `use line::emit_reconcile_read_buffer;` (a free fn needs the import); `motion.py` →
      wiring only.
- [x] Repoint the 3 citations: `stdlib/21_fs-host-errors.md` `lower_fs_read_text_path_helper` →
      `fs/gen_atomic_write/read_path.rs`; `stdlib/23_fs-memory-files.md` `lower_fs_seek_helper` →
      `fs/gen_read_write/position.rs`, `lower_fs_read_line_helper` → `fs/gen_read_write/line.rs`.
- [x] The `SELF_UPDATE_TABLE` proof string → `builtins/fs/gen_atomic_write/read_path.rs` (§4.5, §4.11).
      Done: in `self_update/tests/census/self_update_table.rs:692`; `self_update::tests` still 11 passed.

Acceptance: the three fs carriers compile from their new layout with no caller changed, and the
fs code they emit is byte-identical on every target.
  Check: the common sequence with built-in `fs`; plus
  `cargo test --test rt_hand_built_record_validation` → passes (it reaches `fs::seek`, which no
  golden calls); step 8 rows: `STALE` for `fs/gen_atomic_write.rs`, `fs/gen_directory.rs`,
  `fs/gen_read_write.rs` only, gone after tighten (est. 20 min total, guess).
  Verified 2026-10-20: `bash scripts/artifact-gate.sh … fs` → 0 diffs; `rt_hand_built_record_validation` → 2 passed;
  `codegen::builtins::fs self_update::tests` → 11 (= G0's 11); list 5018 (= N); check warnings 0 (= W);
  `spec_citations_resolve` ok; guards → 9, 8, 7 + 1 ignored; sweep → 0; budget → the three STALE rows (1197, 1028, 1657)
  before, none after; census → atomic_write 260/457/486, directory 361/321/352, read_write 683/553/430.
Commit: c8c5b1cab

### Phase 2 — crypto EC lowerings (pure motion)

- [x] `crypto/func_generate.rs` → `crypto/func_generate/{mod,macos,linux,windows}.rs` per §4.2;
      the 12 `pub(super)` edits.
- [x] `crypto/func_sign.rs` → `crypto/func_sign/{mod,macos,linux,windows}.rs` per §4.3; 3 `pub(super)`.
- [x] `crypto/func_verify.rs` → `crypto/func_verify/{mod,macos,linux,windows}.rs` per §4.4; 3 `pub(super)`.
- [x] `tests/runtime/rt_debug_soak.rs` comment (`grep -n 'crypto/func_generate.rs' tests/runtime/rt_debug_soak.rs`; line 707 at `ce7a995ca`, B keeps it in the root) → `crypto/func_generate/mod.rs` (§4.11).

Acceptance: EC key generation, signing and verification emit byte-identical code on every target
(each backend's emitter is reached by its target's golden).
  Check: the common sequence with built-in `crypto`; plus
  `bash scripts/test-accept.sh target/release/mfb /tmp/s-accept crypto-ec-valid` → the fixture
  passes with no artifact diff (it is outside `tests/byte-identity/`) (est. 2 min, guess); step 8 rows: `STALE`
  for the three crypto files only, gone after tighten (est. 25 min total, guess).
  Done: `codesplit.py` by §4.2–4.4's ranges, each `mod.rs` reaching its parts through private globs; `pub(super)` from
  the compiler's errors → func_generate 14 (§4.2's 12 + the macOS frame slots `NUMVAL`, `PUBLEN`, which
  `lower_generate` also writes), func_sign 3, func_verify 3; `motion.py` → wiring and visibility only; citations 0;
  `rt_debug_soak.rs:710` → `crypto/func_generate/mod.rs`.
  Verified 2026-10-20: `bash scripts/artifact-gate.sh … crypto` → 0 diffs; `test-accept.sh … crypto-ec-valid` → 1
  passed; `codegen::builtins::crypto` → 18 (= G0's 18); list 5018; check warnings 0; `spec_citations_resolve` ok;
  guards → 9, 8, 7 + 1 ignored; sweep → 0; budget → three STALE rows (1897, 1635, 1487) before, none after; census →
  generate 477/409/559/461, sign 452/263/445/484, verify 365/244/374/513.
Commit: 1f1e016b8

### Phase 3 — math and money kernels (pure motion, impl splits)

- [x] `math/gen_math.rs` → `math/gen_math/{mod,arrays,scalar,rng}.rs` per §4.8; the 18
      `pub(super)` edits.
- [x] `money/gen_fixed_math.rs` → `money/gen_fixed_math/{mod,trig,exp_log}.rs` per §4.9; 0
      visibility edits; the inline `mod tests` stays in `mod.rs`.
- [x] Repoint the 3 `gen_math.rs` citations (incl. the **pre-existing stale** `gen_math.rs:883` →
      `gen_math/rng.rs:lower_math_seed`, evidence §4.8) and the 11 `gen_fixed_math.rs` citations (§4.9).
- [x] The 27 comment repoints of §4.11 marked Phase 3.

Acceptance: float math and fixed-point kernels emit byte-identical code; the overflow-label guard
still scans the three moved labels.
  Check: the common sequence with built-ins `math`, `money`, `vector`, `collections` (`math::sum`
  reaches `FloatInfinityError`); plus
  `cargo test -p mfb --bin mfb no_overflow_label_returns_through_the_result_tag_register` → passes,
  and its scanned-file count includes `gen_math/scalar.rs`, `gen_fixed_math/trig.rs`,
  `gen_fixed_math/exp_log.rs` (`grep -rln 'abi::label(&.*overflow' src/codegen/builtins/math/gen_math src/codegen/builtins/money/gen_fixed_math`
  → those 3 files); `cargo test -p mfb --bin mfb codegen::builtins::money` → the inline test
  passes; step 8 rows: `STALE` for `math/gen_math.rs` and `money/gen_fixed_math.rs` only, gone
  after tighten (est. 25 min total, guess).
  Done: `items.py split` (method-level) → gen_math arrays 11, scalar 10, rng 3 methods with exactly the 18
  `pub(super)`; gen_fixed_math trig 10, exp_log 5 (incl. `emit_fixed_pow_tiny_reciprocal`, added since planning) with
  0 visibility edits; citations → 2 + the stale `gen_math.rs:883` → `gen_math/rng.rs:lower_math_seed` by hand, 12
  fixed-math (one added since); comments → the 21 `func_*.rs` carrier lines + the three `money/gen_fixed_math.rs`
  mentions + `declared_errors_all.rs`, `builder_simd_float_math.rs`, `float_parse_table.rs`.
  Verified 2026-10-20: G2 (`bash`) math, money, vector, collections → 0 diffs; overflow scan + `codegen::builtins::
  {money,math}` → 13 passed (= G0's 13); the three label files are `gen_math/scalar.rs`, `gen_fixed_math/trig.rs`,
  `gen_fixed_math/exp_log.rs`; list 5018; check warnings 0; `spec_citations_resolve` ok; guards → 9, 8, 7 + 1 ignored;
  sweep → 0; budget → two STALE rows (1136, 1414) before, none after; census → gen_math 200/298/508/154,
  gen_fixed_math 264/60 + 669/489.
Commit: 57bfaebaa

### Phase 4 — canvas render source (`concat!` rebuild)

- [x] Save `git show HEAD:src/codegen/builtins/canvas/helper_render.rs > /tmp/hr_old.rs`.
- [x] `canvas/helper_render.rs` → `canvas/helper_render/{mod,scene,gpu_groups,gpu_draw_list,gpu_passes,render_loop,present}.rs`
      per §4.1; `RENDER_METAL_TEMPLATE` = `concat!` of the three macro pieces; the 14 `pub(super)`
      consts and 3 macro re-exports; `#[cfg(test)] mod tests;` still resolves to `helper_render/tests/mod.rs`.
- [x] Run the §4.1 text-identity check → `True` and equal lengths.
- [x] Repoint the 20 `helper_render.rs` citations (§4.1).
- [x] The 6 sweep items of §4.11 marked Phase 4 (two `.ai/canvas-threading.md` lines, three
      comments, and the **pre-existing stale** `audio/func_render.rs:4`).

Acceptance: every canvas program's companion source and native code are byte-identical.
  Check: §4.1 text-identity → `True`; steps 1–6 and 8–10 of the common sequence; `cargo build --release`
  then `bash scripts/test-accept.sh target/release/mfb /tmp/s-accept app-mouse-surface` → passes
  with no artifact diff (the only golden fixture that builds a canvas program; no
  `artifact-gate.sh <builtin>` selector reaches canvas) (est. 3 min, guess);
  `cargo test -p mfb --bin mfb helper_render` → the 11 `helper_render` tests pass (incl.
  `every_cap_token_in_the_render_source_is_generated`); `cargo test --test rt_canvas_debug_hooks --test rt_canvas_pointer`
  → pass (the `_DEBUG` render-loop and pointer-script variants) (est. 5 min, guess); step 8
  rows: `STALE` for `canvas/helper_render.rs` only, gone after tighten (est. 25 min total, guess).
  Done: `s4render.py` cut the template text at `' plan-116-H: the GPU draw list` and `' The frame buffer's item region`
  (each after a blank line) and moved the other items by line range; the pre-split copy is
  `target/p176/hr_old.rs` (this machine wipes `/tmp`). `pub(super)` on exactly the 14 consts; 3 macro re-exports.
  Cites → 13 (`citesweep.py`) + 8 MFBASIC `FUNC` names (`mfbcites.py`) = 21 (one added since); the six §4.11 items.
  Verified 2026-10-20: §4.1 text identity → `True 76092 76092` (and again after `cargo fmt`); `test-accept.sh …
  app-mouse-surface` → 1 passed; R-BID's ten canvas scenes (an extra canvas check) → `diff` empty; `helper_render` →
  13 passed (= G0's 13 for that filter); `rt_canvas_debug_hooks` → 3, `rt_canvas_pointer` → 2 passed; list 5018; check
  warnings 0; `spec_citations_resolve` ok + `wordcheck.py` 0 missing; guards → 9, 8, 7 + 1 ignored; sweep → only `'`
  raw-string lines; budget → one STALE row (2204) before, none after; census → mod 98/2, scene 103, gpu_groups 275,
  gpu_draw_list 622, gpu_passes 582, render_loop 406, present 151.
Commit: d30f7a371

### Phase 5 — Extract canvas `register` sub-registrars (largest blast radius last)

- [x] Before editing: `cargo build --release && target/release/mfb man canvas > /tmp/s-man-before.txt`.
- [x] Create `canvas/types_values.rs`, `types_paint.rs`, `types_items.rs`, `types_motion.rs`,
      `types_groups_curves.rs`, each `use super::*;` + `pub(super) fn register(pkg: &mut RegistryPackage)`
      holding lines 280-741, 743-1079, 1081-1620, 1622-2075, 2077-2378 of `canvas/mod.rs`
      verbatim (§4.10), including their banners and leading comments.
- [x] `canvas/mod.rs`: replace the five runs by the five calls, in order, between `add_imports`
      and the Resources banner; add the five private `mod types_*;` decls; edit the `register`
      doc sentence (§4.10).
- [x] `app/06_canvas.md` `[[…/canvas/mod.rs:JoinStyle]]` → `[[src/codegen/builtins/canvas/types_values.rs:JoinStyle]]`.
- [x] `cargo clippy --all-targets 2>&1 | grep -c '^warning'` → the count recorded before the phase.
- [x] `python3 -I tools/loc-census/census.py --file src/codegen/builtins/canvas/mod.rs` → code ≤ 900;
      record the largest new fn's LOC (`types_items::register`) here.
      Done: `canvas/mod.rs` → 341 code; the largest new file is `types_items.rs` at 530 (its one fn).

Acceptance: the canvas registry — every record, enum, union and resource, in order — and the
companion source it renders are unchanged, so every canvas program's code is byte-identical.
  Check: steps 1–6 and 8–9 of the common sequence; `cargo build --release` then
  `bash scripts/test-accept.sh target/release/mfb /tmp/s-accept app-mouse-surface` → passes with
  no artifact diff (est. 3 min, guess); `cargo test -p mfb --bin mfb codegen::builtins::canvas`
  → the 14 canvas registry tests (`canvas/tests/mod.rs`, incl. `draw_item_variant_set_is_frozen`,
  `reassembled_source_parses`, `canvas_types_are_builtin_types`) and the `helper_render` tests
  pass (est. 3 min, guess); `cargo test -p mfb --bin mfb registry` → passes (est. 3 min, guess);
  `target/release/mfb man canvas > /tmp/s-man-after.txt; diff /tmp/s-man-before.txt /tmp/s-man-after.txt`
  → no output (the man page renders the registry's types in registration order) (est. 30 s); step 8 rows: `TIGHTEN` for `canvas/mod.rs` (its ceiling is now removable),
  gone after tighten (est. 25 min total, guess).
  Done: `s5extract.py` located each run by its last registered type (`FillRule`, `Paint`, `DrawItem`, `AutoPaint`,
  `DrawLayer`) — lines 289-779, 781-1117, 1119-1684, 1686-2139, 2141-2442 at this commit — and moved each whole, with
  its banner and leading comments, into `types_*::register`; the five calls replace them in order before the
  Resources banner. A check that the five bodies equal the original lines (blank separators aside) → `VERBATIM 2154
  2154`. The `register` doc sentence and the `JoinStyle` cite as §4.10.
  Verified 2026-10-20: `mfb man canvas` before/after → identical (237 lines); `test-accept.sh … app-mouse-surface` → 1
  passed; R-BID's ten canvas scenes → `diff` empty; `codegen::builtins::canvas` → 50, `registry` → 107 (= G0's 107);
  list 5018; check warnings 0; clippy 37065; `spec_citations_resolve` ok; guards → 9, 8, 7 + 1 ignored; budget →
  `TIGHTEN code 341 (ceiling 2371) … canvas/mod.rs`, removed by the tighten; census → types_values 468, types_paint 321,
  types_items 530, types_motion 444, types_groups_curves 292.
Commit: —

## Validation Plan

- Tests: no test is added or removed. The canvas registry tests (14), `helper_render` tests (11)
  and `gen_fixed_math`'s inline test run in the phases that move their code; the `mfb` binary's
  test count is equal before and after every phase.
- Coverage check: crypto, fs, math, money and vector lowering are reached by their
  `tests/byte-identity/<builtin>` fixtures (§2 Verified properties), plus `crypto-ec-valid`.
  Not reached by a golden: `fs::seek` (covered by `rt_hand_built_record_validation`), and canvas
  except through `tests/syntax/app/app-mouse-surface` (run in Phases 4–5 and in the full gate).
  The §4.1 text-identity check proves the canvas template exactly, independent of coverage.
- Runtime proof: `rt_canvas_debug_hooks`, `rt_canvas_pointer` and `rt_hand_built_record_validation`
  build and run real programs through the moved canvas and fs code.
- Doc sync: no `mfb spec` or `mfb man` contract changes; only citations move (6 spec files,
  `spec_citations_resolve`, plus the hand-repointed `JoinStyle`), the pre-existing stale
  `gen_math.rs:883` citation is fixed, and `.ai/canvas-threading.md` is repointed.
- Final gate (run ONCE, after Phase 5): `cargo build --release` then
  `sh scripts/artifact-gate.sh target/release/mfb all` → 0 diffs (15–20 min; code motion in
  codegen can change emission for any fixture on any target, and only the full gate builds all
  targets plus the canvas fixture). Then
  `cargo test --test file_size_budget 2>&1 | grep -cE 'src/codegen/builtins/(canvas/(helper_render|mod|types_)|crypto/func_(generate|sign|verify)|fs/gen_(atomic_write|directory|read_write)|math/gen_math|money/gen_fixed_math)'`
  → 0, and `grep -cE '^ceiling .*(canvas/helper_render|canvas/mod\.rs|crypto/func_(generate|sign|verify)|fs/gen_(atomic_write|directory|read_write)|math/gen_math|money/gen_fixed_math)' tests/guards/file_size_ceilings.txt`
  → 0. The full workspace suite runs once, in plan-176-V's Validation Plan.

## Decisions

- Rule revised — owner, 2026-10-18 (replaces the 2026-10-14 code ≤ 1000 / test ≤ 250 rule): three
  kinds, non-comment non-blank LOC, scope `.rs`/`.mfb` under `src/`, `packages/`, `examples/`,
  `repository/`, `tests/`: `code` (a non-test-only file's code) ≤ 1000 → split into modules;
  `inline` (`#[cfg(test)]` / `TESTING … END TESTING` LOC inside a non-test-only file) ≤ 250 → moved
  out to a test-only file (one `tests/mod.rs` unless > 1000); `test` (a test-only file's total) ≤ 1000
  → split into modules. Parts target ≤ 900. Ceiling rows are `ceiling <code|inline|test> <n> <path>`.
  No exemptions; only `GENERATED`-banner files are skipped; the guard warns with a banner and never
  fails; a new module is always an acceptable split. In S: `canvas/mod.rs`'s inline tests moved to
  `canvas/tests/mod.rs` in plan-176-H; `gen_fixed_math`'s 60 stay inline.
- A single function over the code limit is decomposed by extract-method refactoring, gated by
  byte-identical `.ncode` — owner, 2026-10-14. In S: canvas `register` → five ordered
  `types_*::register` sub-registrars (§4.10). There are no exemptions (rule revised 2026-10-18).
- `RENDER_METAL_TEMPLATE` is rebuilt with `concat!` from three `macro_rules!` pieces cut at
  MFBASIC top-level boundaries — author, 2026-10-14, following the `RENDER_LOOP` precedent (§4.1).
- The extract-method refactor is its own phase, after every pure-motion phase — author (Phase 5).
- The sub-registrar files are named `types_*` and the five calls sit between `add_imports` and
  the Resources banner — author (§3 rejected alternatives, §4.10).
- The overflow-label guard's directory rule is owned by plan-176-O Phase 1, the first letter to
  split a `builder_*` file — author (§3).
- Code comments that name a moved file are repointed to the file holding the named symbol, or
  to the directory module when they name the module as a whole — author (§4.11).
- Pre-existing stale references found in S's sweep are fixed in S: `stdlib/07_math-rng.md`
  `gen_math.rs:883`, `audio/func_render.rs:4` — owner rule (AGENTS.md "never leave a bug"),
  applied by the author.

## Corrections

- 2026-10-20 — Every `sh scripts/artifact-gate.sh` runs as `bash` (the script needs bash).

- 2026-10-18: Rule revised by the owner (code ≤ 1000, inline ≤ 250, test-only file ≤ 1000, no exemptions); file list and split design unchanged; rule restated in intro, common step 9 and Decisions; H's canvas tests are one `canvas/tests/mod.rs`, `helper_render/tests/mod.rs` (308) and `tests/declared_errors_all.rs` (797) are no longer moved or split by H.

## Summary

The risk is in Phases 4 and 5. Phase 4 rebuilds a 1464-line MFBASIC literal from three pieces;
an exact text-equality check plus the canvas fixture prove it, because no scoped gate sees canvas.
Phase 5 moves 45 registry statements, verbatim and in order, into five sub-registrars called in
source position; the registry's insertion order is the only invariant, and the canvas registry
tests, the companion-source parse test and the canvas fixture check it. Phases 1–3 are pure motion
whose hazards are bookkeeping: 43 spec citations (one already stale), about 30 comment mentions,
and one guard whose corpus is chosen by file name. Untouched: every emitted byte, every crate path
callers use, every test name, the canvas test modules (plan-176-H's), the other canvas files
(plan-176-R's) and the bug documents.

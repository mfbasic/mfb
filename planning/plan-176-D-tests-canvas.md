# plan-176-D: Split tests/canvas (6 integration-test files)

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-C

This sub-plan splits the 6 integration-test files under `tests/canvas/` that are over the
test-only file limit (1000 LOC): `rt_canvas_curves.rs`, `rt_canvas_font.rs`,
`rt_canvas_golden.rs`, `rt_canvas_metal.rs`, `rt_canvas_particles.rs` and
`rt_canvas_rasteriser.rs`. Together they hold 14,845 test LOC and 344 `#[test]` fns. Each root
stays the `[[test]]` target and gains 1–6 topic modules under `tests/canvas/<name>/`, declared
with `#[path]`, the layout plan-176-A verified. Every part is ≤ 900 LOC (headroom under 1000).
It is pure code motion. The only edits beyond motion are decided ones:

- 56 `include_str!("scenes/…")` paths in the two `rt_canvas_metal` topics gain a `../`;
- fixing two pre-existing stale `tests/rt_canvas_*.rs` paths (`.ai/testing-gates.md:332`,
  `scripts/man-run-examples.sh:260`);
- repointing one live plan line (`planning/plan-175-B-canvas-draw-node-table.md:260`) whose
  `draws=` assertions move into a topic, if that plan is still open when Phase 4 lands.

No shared helper changes visibility: every helper used by more than one part stays in the root,
and private root items are visible to child modules. No spec citation moves: the five
`[[tests/canvas/…]]` citations this letter owns name symbols that stay in their roots.

**Single outcome:** after D lands, `cargo test --test file_size_budget` prints no row for any
path under `tests/canvas/` that this letter owns, and `tests/guards/file_size_ceilings.txt` has no
`ceiling` row for them.

References:

- `planning/plan-176-A-file-size-guard.md` (the rule, the counting rules, the guard and its
  banner, the ceiling-file format `ceiling <code|inline|test> <n> <path>`, the `#[path]` layout)
  and `planning/plan-176-B-tests-runtime-a.md` (the gate kit K1–K5, restated in §3).
- `.ai/build-tooling.md` § "Splitting an over-budget file (plan-176)".
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations". `spec_citations_resolve`
  checks citations at file level only, so a moved symbol whose root file still exists passes
  silently; K6 below greps each cited symbol. The file also has a stale path at :332, fixed in
  Phase 1.
- `.ai/canvas-threading.md` (:926 names `rt_canvas_metal.rs`, :1033 names
  `tests/canvas/rt_canvas_golden.rs`; both stay valid).
- `.ai/specifications.md` (citation form `[[path:symbol]]`).
- Spec topic: `src/docs/spec/app/06_canvas.md`, D citations at :266, :1536, :1548 (rasteriser),
  :1628 (golden), :1629 (metal).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-C is complete and archived | `ls planning/plan-176-C-* planning/completed/plan-176-C-*` → only `planning/completed/plan-176-C-tests-runtime-b.md` | MET (2026-10-18, HEAD d4c18d5ea: `ls planning/completed/ \| grep plan-176-C` → `plan-176-C-tests-runtime-b.md`; `ls planning \| grep plan-176-C` → none) |
| The guard and the oracle exist (plan-176-A) | `cargo test --test file_size_budget` → passes; `python3 -I tools/loc-census/census.py --file tests/canvas/rt_canvas_rasteriser.rs` → `0 4712` | MET (2026-10-18: guard `test result: ok. 35 passed`; the census → `0 4712`) |
| This letter's files have no uncommitted changes | `git status --porcelain -- tests/canvas/rt_canvas_curves.rs tests/canvas/rt_canvas_font.rs tests/canvas/rt_canvas_golden.rs tests/canvas/rt_canvas_metal.rs tests/canvas/rt_canvas_particles.rs tests/canvas/rt_canvas_rasteriser.rs tests/canvas/scenes src/docs/spec/app/06_canvas.md scripts/man-run-examples.sh .ai/testing-gates.md planning/plan-175-B-canvas-draw-node-table.md tests/guards/file_size_ceilings.txt` → empty | MET (2026-10-18, HEAD d4c18d5ea: empty) |
| The six files still measure as mapped (§2). Any drift means re-running the part sums before splitting | `for f in curves font golden metal particles rasteriser; do python3 -I tools/loc-census/census.py --file tests/canvas/rt_canvas_$f.rs; grep -cE '^\s*#\[test' tests/canvas/rt_canvas_$f.rs; done` → `0 2432` 40, `0 3547` 60, `0 1182` 28, `0 1758` 74, `0 1214` 29, `0 4712` 113 | MET (2026-10-18, HEAD d4c18d5ea: the command → exactly those six LOC pairs and counts) |

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

- Every file of this letter, and every file it creates, is ≤ 1000 test LOC by
  `tools/loc-census/census.py --file`; every part is planned at ≤ 900.
- For every one of the 6 targets, the sorted list of test names from `-- --list`, with module
  prefixes stripped, is identical before and after. Every target passes `cargo test --test <t>`
  on the macOS development host, which is the Metal host.
- Every `[[tests/canvas/…:symbol]]` citation for these six targets names the file that holds the
  symbol (all five stay in their roots; K6 proves it).

### Non-goals (explicit constraints)

- No behavior change: no test body, assertion, scene source, golden PNG or `#[ignore]` reason
  changes. No string constant changes value; `MODEL_DRIVER` and every other scene/program
  constant moves verbatim, inline.
- No `[[test]]` target added, renamed or re-pathed. `Cargo.toml` is not edited.
- No test fn renamed, split or merged; the test count of every target is unchanged.
  `measure_the_ellipse_newton_iteration_count` moves whole, with its nested fns.
- `tests/canvas/scenes/*.mfb` are not touched. The `GROUPS` and `POLYLINES` consts stay in the
  `rt_canvas_golden` root, unindented, because `scripts/test-canvas-vulkan.sh` extracts them
  with `sed`.
- No helper changes visibility. No helper moves into `tests/common/`.

## 2. Current State

### Files and measured sizes

Test LOC are from the planning oracle at HEAD `ce7a995ca`
(`python3 -I /tmp/loc/census.py "$PWD"` → `/tmp/loc/census.json`; after A it is
`tools/loc-census/census.py --file <path>`). Test counts are `grep -cE '^\s*#\[test' <file>`.
Part sizes are sums of top-level item spans counted by the same oracle rules
(`/tmp/loc/rw-D/items.py` splits a file into top-level items; their sum equals the census total
for all six files), plus the added lines: 2 per `#[path]` declaration in the root, 1 per
`use super::*;` in a topic. Items are assigned to the part of their tests; a helper used by more
than one part stays in the root (`/tmp/loc/rw-D/assign3.py`).

| File | Test LOC | Tests | New files | Largest part after |
|---|---|---|---|---|
| `tests/canvas/rt_canvas_curves.rs` | 2432 | 40 | 3 | 772 |
| `tests/canvas/rt_canvas_font.rs` | 3547 | 60 | 4 | 775 |
| `tests/canvas/rt_canvas_golden.rs` | 1182 | 28 | 1 | 788 |
| `tests/canvas/rt_canvas_metal.rs` | 1758 | 74 | 2 | 731 |
| `tests/canvas/rt_canvas_particles.rs` | 1214 | 29 | 1 | 669 |
| `tests/canvas/rt_canvas_rasteriser.rs` | 4712 | 113 | 6 | 787 |
| **Total (6 files)** | **14,845** | **344** | **17 `.rs`** | |

Measurement history: the test names and layouts of `rt_canvas_curves`, `rt_canvas_golden` and
`rt_canvas_particles` are unchanged since the `17a9b5799` map (`/tmp/loc/maps/D.md`), and their
LOC are identical. `rt_canvas_font` grew from plan-162 (face index, kerning, font lists,
spacing, decoration, carets, ligatures) and bug-1091; its layout below is re-mapped at
`ce7a995ca`. `rt_canvas_metal` gained three tests (`a_tile_edge_pixel_samples_inside_its_source_rectangle_on_metal`,
`banded_shapes_in_shrinking_groups_match_the_software_oracle_on_metal`,
`magnified_arcs_match_the_software_oracle_on_metal`) and three scene includes;
`rt_canvas_rasteriser` gained four butt-arc tests (bug-1088).

### Verified properties

- **Layout, globs and warnings.** These are the same facts as plan-176-B §2, from the same probes:
  the plan-176-A `#[path]` probe; `/tmp/loc_bcd_probe` (a topic sees root imports, bare `common`
  helpers and root items with no warning); `/tmp/loc/probe2` (glob cycles clean, printed `6`).
  `use super::*;` in a file that references no parent item warns; every topic below calls a root
  helper or `common`, so every topic starts with `use super::*;`.
- **Private root items, fields and methods are visible to child modules.** Rust privacy is per
  module and its descendants, so `Scene`'s fields and both `impl Scene` blocks
  (`rt_canvas_curves`), `Program` and its `impl Drop` (`rt_canvas_metal`) and `FontRun`
  (`rt_canvas_font`) need no `pub(super)` while they stay in the root, and they do.
- **Every D test compiles on the macOS development host.** `grep -nE '^\s*#!?\[cfg' tests/canvas/rt_canvas_{curves,font,golden,metal,particles,rasteriser}.rs`
  → two `#[cfg(unix)]` tests in `rt_canvas_curves` (`canvas_curve_shapes_are_hit_where_they_are_drawn`,
  `canvas_picture_on_a_poly_curve_draws_nothing`) and four `#[cfg(target_os = "macos")]` tests in
  `rt_canvas_font` (`a_system_collection_face_loads_by_post_script_name`,
  `face_index_answers_real_system_fonts_exactly`, `a_real_font_kerns_as_harfbuzz_does`,
  `a_real_font_ligates_as_harfbuzz_does`). The attributes move with their fns. No crate-level
  `#![cfg]` in these six files. Metal tests need the macOS host, and that is the host the gates
  run on. Two tests are `#[ignore]`d design measurements in `rt_canvas_rasteriser`
  (`measure_the_transformed_distance_correction`, `measure_the_ellipse_newton_iteration_count`);
  `--list` lists them, so K1/K2 cover them.
- **Scene includes are relative to the including file.** `grep -c 'include_str!("scenes/'` →
  `rt_canvas_metal.rs` 56, `rt_canvas_golden.rs` 4, the other four 0. The 4 golden includes
  (`CORNERS`, `PARTICLES`, `PICTURE_SHAPES`, `GROUP_AFFINE`) stay in the golden root, unchanged.
  All 56 metal includes move one directory deeper (32 into `scenes_over_black.rs`, 24 into
  `scenes_and_passes.rs`; §4.2) and become `"../scenes/…"`; the root keeps none.
- **`scripts/test-canvas-vulkan.sh` extracts two scenes from the golden root.** At :541 and :682,
  `sed -n '/^const GROUPS: &str = r#"/,/^"#;$/p' tests/canvas/rt_canvas_golden.rs` (and the same
  for `POLYLINES`) print 26 and 43 lines today. Both consts stay in the root, unindented.
- **No indivisible unit is near the limit.** The largest single items are `rt_canvas_particles`'
  `MODEL_DRIVER` raw string (250 LOC, L256–505, read only by the `{MODEL_DRIVER}` capture at
  L512) and `rt_canvas_rasteriser`'s `#[ignore]`d `measure_the_ellipse_newton_iteration_count`
  (259 LOC including its four nested fns `truth`, `solve`, `bisect`, `steps`). Both move whole
  into a part of ≤ 900, so neither needs an extraction.
- **The five spec citations** (`grep -rnoE '\[\[tests/canvas/rt_canvas_(curves|font|golden|metal|particles|rasteriser)\.rs[^]]*\]\]' src/docs .ai`)
  are exactly `06_canvas.md:266` (`path_placement_a_clipped_polygon_moves_whole_with_its_position`),
  `:1536` (`group_affine_a_nested_group_composes_with_its_parent`), `:1548`
  (`group_affine_a_scaled_group_scales_its_strokes`), `:1628`
  (`the_gpu_draws_the_group_affine_scene_the_reference_shows`) and `:1629`
  (`a_group_with_a_matrix_draws_on_metal_and_matches_the_software_oracle`). The layout in §4
  keeps all five symbols in their roots. No `mfb man` page names a D file
  (`grep -rn 'tests/canvas/rt_' src/docs/man` → no hit).
- **Two references are stale today.** `.ai/testing-gates.md:332` names `tests/rt_canvas_metal.rs`,
  and `scripts/man-run-examples.sh:260` names `tests/rt_canvas_font.rs`. `ls` of both →
  `No such file or directory`. These are the only `tests/rt_canvas_*.rs` references in
  `.ai scripts AGENTS.md src/docs .github` (`grep -rn 'tests/rt_canvas'`).
- **Live documents naming a symbol in a D file** (`grep -rnoE 'rt_canvas_(curves|font|golden|metal|particles|rasteriser)\.rs\`?(::|:|\`'"'"'s )' .ai scripts AGENTS.md src/docs .github planning/*.md bugs/*.md`,
  minus plan-176): the five citations above and `planning/plan-175-B-canvas-draw-node-table.md:260`
  ("`tests/canvas/rt_canvas_rasteriser.rs`'s `draws=` assertions"). Those assertions live in
  `scene_draws_shares_one_base_between_a_diamonds_two_draws` (L5688–5795 at `ce7a995ca`), which
  moves to `rt_canvas_rasteriser/groups.rs`. Every other live reference names a root path or a
  target (`scripts/remote-common.sh:47` and `scripts/test-canvas-vulkan.sh:88` name the font
  fixture builder in `tests/canvas/rt_canvas_font.rs`, and the builder stays in that root;
  `planning/plan-175-C-canvas-group-node-instancing.md:147` adds tests to the rasteriser target) and stays valid.
- No D file respawns itself, reads its own source, or uses `module_path!`/`file!()`
  (`grep -nE 'current_exe|--exact|module_path!|file!\(\)'` → none). No D path is in
  `scripts/coverage-exceptions.txt` or a guard allowlist. No `tests/canvas/<name>/` directory
  exists yet. `tests/guards/test_targets_registered.rs` scans only `tests/<dir>/*.rs`, so the new
  `tests/canvas/<name>/*.rs` files need no stanza.

## 3. Design Overview

**Conventions (every file)** are plan-176-B §3's conventions:

- The root stays the `[[test]]` target. It keeps `#[path = "../common/mod.rs"] mod common;`, its
  `use` lines, and every helper §4 lists for it. Each child is declared as
  `#[path = "<name>/<child>.rs"] mod <child>;` (2 LOC each, already counted in §4's root numbers).
- Every child is a topic: it starts with `use super::*;`, keeps its local helpers private, and
  changes no visibility. There are no support files: a helper used by more than one part stays
  in the root, where every child sees it.
- Code moves verbatim with doc comments and attributes (`#[test]`, `#[ignore = …]`, `#[cfg(…)]`).
  Item and test names are authoritative if lines drift from `ce7a995ca`.
- Parts merge neighbouring topics of the `17a9b5799` map (`/tmp/loc/maps/D.md`) by
  responsibility; §4 names the map topics each part absorbs, so the map's per-topic test lists
  remain the inventory.

**Gate class: test-motion,** using plan-176-B's kit with `/tmp/176D-` prefixes and `tests/canvas/`
paths:

- **K1 list before**:
  `for t in $T; do cargo test -q --test $t -- --list 2>/dev/null | grep ': test$' | sed -E 's/^([a-z0-9_]+::)+//' | sort > /tmp/176D-$t.list; done; cat $(for t in $T; do echo /tmp/176D-$t.list; done) | wc -l` → the phase total.
- **K2 list after**:
  `for t in $T; do cargo test -q --test $t -- --list 2>/dev/null | grep ': test$' | sed -E 's/^([a-z0-9_]+::)+//' | sort | diff -q /tmp/176D-$t.list - >/dev/null || echo "DIFF $t"; done` → no output.
- **K3 run**: `cargo test $(for t in $T; do printf -- '--test %s ' $t; done)` → each `test result: ok`, 0 failed.
- **K4 warnings**: `cargo check --all-targets 2>&1 | grep -E -- "--> tests/canvas/$RE(\.rs|/)"` → no output.
- **K5 budget**: `cargo test --test file_size_budget 2>&1 | grep -E "tests/canvas/$RE(\.rs|/)"` → only
  `TIGHTEN` rows of kind `test`, one per root (its `ceiling test` entry is now removable). Then
  `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`. Then
  `git diff -U0 tests/guards/file_size_ceilings.txt | grep -E '^[-+]ceiling' | grep -vE "tests/canvas/$RE(\.rs|/)"` → no output.
  Re-running the first grep → no output.
- **K6 citations** (phases whose roots hold a cited symbol): `cargo test -p mfb --bin mfb spec_citations_resolve` → ok,
  and for each citation the phase owns, `grep -n 'fn <symbol>' tests/canvas/<root>.rs` → one hit,
  because the resolver itself does not check symbols.

**Where the risk concentrates.** `rt_canvas_rasteriser` has 113 tests in 7 parts and is the
largest split, so it goes last. `rt_canvas_font` is the one layout re-mapped at `ce7a995ca`,
with the most cross-part helpers (the sfnt/OpenType table builders), so it has its own phase. The
metal suite carries 56 scene includes; a wrong include path is a compile error. A moved
`GROUPS`/`POLYLINES` would break only the Linux Vulkan script, so Phase 1 checks the `sed` output.

**Rejected alternatives.**

- *Extracting `MODEL_DRIVER` to an `include_str!` `.mfb`, and hoisting the Newton measurement's
  nested fns into a sibling module.* Both units fit, whole, inside a part of ≤ 900, so pure
  motion suffices and the extra non-motion edits buy nothing.
- *Support modules with `pub(super)` items* (one per helper family, e.g. font fixture, curve
  model, render helpers). Keeping cross-part helpers in the root keeps every root ≤ 788 without
  any visibility edit, and keeps the font fixture builder at the path two scripts cite.
- *Moving the scene strings of `rt_canvas_metal` into `tests/canvas/scenes/`*. That is not
  needed for size, and the GPU-rows scripts compile every `scenes/*.mfb`, so a new file there
  becomes a new script case.
- *Keeping the metal includes in the root to avoid the 56 path edits.* The five map topics
  that hold includes (stars_fills_and_strokes, shapes_over_black, paths_and_passes, large_scenes,
  curves_and_buffer_passes, plus the three new tests) total 911 LOC of items, more than a whole
  part before the root's own helpers are added.
- Helper promotion into `tests/common/`, per-topic `[[test]]` targets, and moving the root are
  rejected for the reasons in plan-176-B §3.

## 4. Detailed Design

### 4.0 Sweeps common to every file

- Spec citations: the five listed in §2. None moves; K6 confirms each symbol is in its root.
- Prose references to D root paths in `.ai/canvas-threading.md` (:926, :1033), `.ai/testing-gates.md`
  (:257, :310), `scripts/test-canvas-gpu-rows.sh:7`, `scripts/test-canvas-vulkan.sh:4,88,516,541,672,682`,
  `scripts/remote-common.sh:47` stay valid, because roots do not move and the font fixture builder
  stays in its root. The exceptions are the two stale paths (Phases 1 and 3) and plan-175-B:260
  (Phase 4).
- Archived documents under `bugs/completed/` and `planning/completed/` record history and are not
  edited.
- Each entry gives the layout (path, LOC, contents), then "Surface", then "Sweeps". LOC are
  "sum of item spans" (§2).

#### 4.1 `tests/canvas/rt_canvas_golden.rs` — 1182 test LOC, 28 tests

- `tests/canvas/rt_canvas_golden.rs` (788) — root: header (`mod common`, `use`s); helpers: WIDTH,
  HEIGHT, golden_path, render_with_font, render, render_gpu_with_font, render_gpu, render_inner,
  `GROUPS` and `POLYLINES` (pinned, unindented, for the Vulkan `sed`); tests kept (19), the map
  topics comparators, blend_and_transforms, endcaps_and_corners, particles_and_pictures and groups:
  smiley_matches_its_reference_exactly, identical_frames_pass_both_comparators,
  a_one_step_perturbation_fails_exact_but_passes_tolerance, a_large_channel_delta_fails_tolerance,
  too_many_small_differences_fail_tolerance, blend_modes_match_their_reference_exactly,
  transforms_match_their_reference_exactly, the_gpu_draws_the_transform_scene_the_reference_shows,
  endcaps_match_their_reference_exactly, the_gpu_draws_the_endcap_scene_the_reference_shows,
  corners_match_their_reference_exactly, particles_match_their_reference_exactly,
  the_gpu_draws_the_particle_scene_the_reference_shows, picture_shapes_match_their_reference_exactly,
  a_scene_containing_a_group_reaches_the_gpu, group_affine_matches_its_reference_exactly,
  the_gpu_draws_the_group_affine_scene_the_reference_shows, groups_match_their_reference_exactly,
  the_gpu_draws_the_group_scene_the_reference_shows; their local consts SMILEY, BLEND_MODES,
  TRANSFORMS, ENDCAPS, CORNERS, PARTICLES, PICTURE_SHAPES, GROUP_SCENE, FLAT_SCENE, GROUP_AFFINE
  stay too; declares `ellipses_gradients_polylines`
- `tests/canvas/rt_canvas_golden/ellipses_gradients_polylines.rs` (397) — 9 tests, the map topics
  ellipses, gradients and polylines: ellipses_match_their_reference_exactly,
  the_gpu_draws_the_ellipse_scene_the_reference_shows, gradients_match_their_reference_exactly,
  the_gpu_draws_the_gradient_scene_the_reference_shows,
  a_frame_past_the_gradient_stop_cap_declines_to_software, a_gradient_on_a_stroked_text_is_ignored,
  polylines_match_their_reference_exactly, a_two_point_polyline_draws_the_line_between_its_points,
  the_gpu_draws_the_polyline_scene_the_reference_shows; local helpers: ELLIPSES, GRADIENTS,
  GRADIENT_OVERFLOW (the polyline tests read the root's `POLYLINES` through `use super::*;`)

Surface: root declares 1 topic as `#[path = "rt_canvas_golden/<name>.rs"] mod <name>;`; target name `rt_canvas_golden` unchanged; no visibility change.
Sweeps: citation `06_canvas.md:1628` `the_gpu_draws_the_group_affine_scene_the_reference_shows` stays in the root (K6) | includes: the 4 `"scenes/…"` includes stay in the root, unchanged; the topic has none | `scripts/test-canvas-vulkan.sh:541,682` `sed` pins: `GROUPS`/`POLYLINES` stay root, unindented. Prose refs (path only): `.ai/canvas-threading.md:1033`, `.ai/testing-gates.md:257,310`, `scripts/test-canvas-vulkan.sh:516,672`.

#### 4.2 `tests/canvas/rt_canvas_metal.rs` — 1758 test LOC, 74 tests

- `tests/canvas/rt_canvas_metal.rs` (731) — root: header (`mod common`, `use`s); helpers: WIDTH,
  HEIGHT, Program (and its `impl Drop`), build, render, render_env, metal_built, stat, lit,
  assert_metal_draws_like_software, assert_metal_draws_within; tests kept (10), the map topics
  primitives, fallback_and_transforms and groups_and_text:
  rectangles_match_the_software_oracle_within_tolerance,
  the_full_primitive_set_matches_the_software_oracle, a_debug_metal_frame_reports_its_own_phases,
  an_unsupported_scene_falls_back_to_the_software_renderer,
  a_group_with_a_matrix_draws_on_metal_and_matches_the_software_oracle,
  an_unevenly_transformed_item_in_a_shrinking_group_keeps_every_covered_pixel,
  every_group_case_matches_the_software_oracle,
  a_diamond_renders_identically_to_the_flat_scene_it_stands_for,
  a_group_naming_an_absent_group_draws_nothing_on_the_gpu,
  every_glyph_of_a_text_run_draws_its_own_bitmap; their local consts RECTANGLES, PRIMITIVES,
  TOO_MANY_EDGES, TURNED_GROUP, HALO_IN_A_SHRINKING_GROUP, GROUPS, DIAMOND_GROUPED, DIAMOND_FLAT,
  ABSENT_GROUP, EMPTY_SCENE, TEXT_LINE stay too (none is an include); declares
  `scenes_over_black`, `scenes_and_passes`
- `tests/canvas/rt_canvas_metal/scenes_over_black.rs` (380) — 33 tests: the map topics
  stars_fills_and_strokes (18) and shapes_over_black (13), plus
  `banded_shapes_in_shrinking_groups_match_the_software_oracle_on_metal` and
  `magnified_arcs_match_the_software_oracle_on_metal`; local helpers: the 32 scene consts STAR_251_97,
  STAR_301_97, STAR_301_2, STAR_301_2_NONZERO, FILL_RULES, SQUARE_CAPS, JOINS, GRADIENTS,
  PATH_CLIPS, CORNERS, TRANSFORMED_CLIPS, BANDED_IN_SHRINKING_GROUPS, MAGNIFIED_ARCS,
  PICTURE_SHAPES, PICTURE_TEXEL_EDGE, DASHES, LARGE_POLYGONS_ON_BLACK, SMALL_POLYGONS_ON_BLACK,
  RECTANGLES_ON_BLACK, CIRCLES_ON_BLACK, ELLIPSES_ON_BLACK, LINES_ON_BLACK, ARCS_ON_BLACK,
  STROKED_POLYGONS_ON_BLACK, POLYLINES_ON_BLACK, GROUPED_POLYGONS_ON_BLACK,
  STROKED_SHAPES_ON_BLACK, RECTANGLES_AT_A_STEP_ON_BLACK, CLIPS_AT_A_STEP_ON_BLACK,
  TRANSFORMED_SHAPES_ON_BLACK, BLENDED_HAIRLINE_STROKE, AUTOPAINT (every one an include)
- `tests/canvas/rt_canvas_metal/scenes_and_passes.rs` (653) — 31 tests: the map topics
  paths_and_passes (4), pictures_and_text_spacing (2), large_scenes (12) and
  curves_and_buffer_passes (12), plus `a_tile_edge_pixel_samples_inside_its_source_rectangle_on_metal`;
  local helpers: TOO_MANY_FRAME_EDGES, PAST_THE_FRAME_EDGE_REGION, POLYLINES, PIVOTED,
  PATH_PLACEMENT, PICTURES, SPACE_THEN_SHAPES, PARTICLES, PARTICLES_GROUPED, FIVE_THOUSAND_QUADS,
  MANY_GRADIENTS, TILEMAP, TILE, TILE_EDGE, BACKGROUND, LARGE_POLYGONS, LAYERED, HUGE_POLYGON,
  OPAQUE_FRAME_EDGES, CURVES, CURVE_STROKES, CURVES_PLACED, CURVE_PICTURES, CURVE_RESIDUALS,
  VERTEX_ON_A_ROW, PIECE_RINGS, MANY_ITEMS, MANY_GRADIENT_STOPS, MANY_TEXELS, PASSES_GROUPED
  (24 of them includes)

Surface: root declares 2 topics as `#[path = "rt_canvas_metal/<name>.rs"] mod <name>;`; target name `rt_canvas_metal` unchanged; no visibility change.
Sweeps: citation `06_canvas.md:1629` `a_group_with_a_matrix_draws_on_metal_and_matches_the_software_oracle` stays in the root (K6) | includes: all 56 move — each `include_str!("scenes/<f>.mfb")` in the two topics becomes `include_str!("../scenes/<f>.mfb")` (`<f>` unchanged; `BANDED_IN_SHRINKING_GROUPS`'s include sits on the line after its `const`, L1801 at `ce7a995ca`); the map's §4.14 const → file list covers the 53 older ones, and the three new ones are `TILE_EDGE` → `tile_edge.mfb`, `BANDED_IN_SHRINKING_GROUPS` → `banded_in_shrinking_groups_on_black.mfb`, `MAGNIFIED_ARCS` → `magnified_arcs_on_black.mfb` | stale `.ai/testing-gates.md:332` `tests/rt_canvas_metal.rs` → `tests/canvas/rt_canvas_metal.rs` | prose refs (path/target only): `scripts/test-canvas-gpu-rows.sh:7`, `scripts/test-canvas-vulkan.sh:4`, `.ai/canvas-threading.md:926`.

#### 4.3 `tests/canvas/rt_canvas_curves.rs` — 2432 test LOC, 40 tests

- `tests/canvas/rt_canvas_curves.rs` (748) — root: header (`mod common`, `use`s); the helpers the
  map split into four support files, all kept here: the driver (EPS, chunk, DRIVER, driver, run,
  value), the curve model (Piece, pieces, marks, Curve and its methods, poly, rational, spell,
  reduce, chain, piece_at, distance, piece_distance, segment_distance, dense_deviation,
  assert_monotone, assert_connected, SQRT_HALF, cubic_s), the scene (WIDTH, HEIGHT, Scene with
  both `impl Scene` blocks, L861–902 and L1133–1175 as of `17a9b5799`, PRELUDE, press, scene,
  scene_on, BLACK, WHITE, BLUE), and the NURBS model (float_source, float_list, point_list, Nurbs
  and its methods); no tests; declares `reduction_and_pieces`, `curve_shapes`, `picture_curves`
- `tests/canvas/rt_canvas_curves/reduction_and_pieces.rs` (772) — 18 tests, the map topics
  reduction (5), chains_and_hashing (6), nurbs (3), flatten (2) and pieces (2); local helpers:
  polyline_distance
- `tests/canvas/rt_canvas_curves/curve_shapes.rs` (678) — 17 tests, the map topics curve_polygon
  (5), gpu_and_joins (4), caps_and_dashes (3) and placement_and_paint (5, including the
  `#[cfg(unix)]` `canvas_curve_shapes_are_hit_where_they_are_drawn`); local helpers: stroke_paint,
  pixel_of, SHAPES
- `tests/canvas/rt_canvas_curves/picture_curves.rs` (243) — 5 tests, the map topics picture_clips
  (2) and picture_extent (3, including the `#[cfg(unix)]` `canvas_picture_on_a_poly_curve_draws_nothing`);
  local helpers: RED, image, coverage_of, edge_gap

Surface: root declares 3 topics as `#[path = "rt_canvas_curves/<name>.rs"] mod <name>;`; target name `rt_canvas_curves` unchanged; no visibility change (the topics use `Curve`/`Nurbs`/`Scene` fields and methods, which are private to the root and visible to its children).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_curves.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | includes: none | `driver`/`helper_source`-style readers of `src/…` use `env!("CARGO_MANIFEST_DIR")` at run time (absolute; unaffected). Refs: none beyond the [[test]] stanza.

#### 4.4 `tests/canvas/rt_canvas_particles.rs` — 1214 test LOC, 29 tests

- `tests/canvas/rt_canvas_particles.rs` (548) — root: header (`mod common`, `use`s); helpers:
  WIDTH, HEIGHT, draw, pixel, lit, stat, SCENE_PRELUDE, scene; tests kept (16), the map topics
  templates (6), template_kinds_and_gpu (5) and damage_and_cache (5); their local helpers TILTED,
  THOUSAND, STREAM_OVER_BACKGROUND, damage, MIXED stay too; declares `lifecycle_and_model`
- `tests/canvas/rt_canvas_particles/lifecycle_and_model.rs` (669) — 13 tests, the map topics
  system_lifecycle (5) and model (8); local helpers: Run, run, run_inner, report, answer, number,
  run_with_font, TEMPLATES, TYPE_SET, helper_source, `MODEL_DRIVER` (the 250-LOC raw string,
  moved verbatim and inline), model, model_case

Surface: root declares 1 topic as `#[path = "rt_canvas_particles/<name>.rs"] mod <name>;`; target name `rt_canvas_particles` unchanged; no visibility change.
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_particles.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | includes: none | `MODEL_DRIVER` and its only reader, the `{MODEL_DRIVER}` capture in `model()`'s `format!` (L512), move into the same file, so the capture still resolves | `helper_source` reads `src/` via `CARGO_MANIFEST_DIR` at run time (unaffected).

#### 4.5 `tests/canvas/rt_canvas_font.rs` — 3547 test LOC, 60 tests

Layout mapped at `ce7a995ca` (line numbers are that tree's). Items are assigned by test range:
L61–228, L545–625, L969–1060 and L1676–2274 are loading, measuring, bounds and collections;
L441–543, L628–968, L1061–1661 and L4316–4396 are drawing; L2287–3391 are the face index and
kerning; L3404–4310 are font lists, shaping, spacing, decoration, carets and ligatures.

- `tests/canvas/rt_canvas_font.rs` (614) — root: header (`mod common`, the `use`s at L20–21 and
  L1672–1674); helpers: run, run_files, ERR_BAD_FONT_FILE, RESOURCE_CLOSED, the fixture builder
  (minimal_truetype, truetype_fixture, truetype_fixture_with — the twelve-glyph TrueType
  `scripts/remote-common.sh:47` and `scripts/test-canvas-vulkan.sh:88` cite), run_with_font,
  run_with_font_frame, the render family (WIDTH, render, render_with, render_env,
  render_env_files, stat, pixel), lit_column_runs, with_table, run_debug_files, and the
  sfnt/OpenType table builders shared by the face-index, kerning and shaping parts
  (replace_table, cmap_with, cmap12, be16, square_record, GID_A, GID_V, with_tables,
  kerning_font, value_size, value_record, coverage1, PairRecord, pair_pos1, lookup,
  layout_table, X_ADVANCE, gpos_format1); no tests; declares `loading_and_measure`, `drawing`,
  `face_index_and_kerning`, `shaping`
- `tests/canvas/rt_canvas_font/loading_and_measure.rs` (752) — 17 tests:
  load_font_accepts_truetype_outlines_and_refuses_every_other_container,
  a_missing_path_is_not_reported_as_a_bad_font, a_second_destroy_font_raises_resource_closed,
  measure_text_scales_the_fonts_own_metrics,
  descent_is_reported_positive_though_the_file_stores_it_negative,
  measure_text_sums_a_mixed_run_in_font_units_then_scales_once,
  measuring_a_closed_font_raises_resource_closed, a_font_past_the_renderers_table_still_measures,
  a_cmap_group_count_is_bounded_by_the_table_that_holds_it,
  a_cmap_group_count_past_the_end_of_the_file_is_bounded_by_the_file,
  a_units_per_em_outside_the_formats_range_is_refused_at_load,
  a_glyph_whose_bitmap_would_exceed_the_raster_cap_is_skipped,
  a_display_sized_glyph_is_well_inside_the_raster_cap, a_collection_loads_its_first_face,
  a_collection_that_runs_past_its_end_is_refused, load_font_chooses_a_collection_face_by_name,
  a_system_collection_face_loads_by_post_script_name (`#[cfg(target_os = "macos")]`); local
  helpers: PROGRAM, ERR_FACE_NOT_FOUND, ERR_NOT_FOUND, MEASURE, FontRun, run_fonts_bounded,
  draw_a, lit_pixels, cmap_subtable_at_eof_claiming_everything, collection, DRAW_A, name_table,
  utf16be, named_face
- `tests/canvas/rt_canvas_font/drawing.rs` (775) — 17 tests:
  text_inside_a_translated_group_draws_at_the_offset, transparency_fades_text_and_stroked_text,
  a_glyph_outline_renders_where_its_own_coordinates_put_it,
  a_composite_glyph_draws_every_placed_component, a_string_advances_the_pen_between_glyphs,
  every_live_font_draws_however_many_are_loaded,
  a_filled_run_places_every_glyph_at_its_own_snapped_pen,
  a_stroked_run_places_every_glyph_at_its_own_pen, verify_mode_agrees_with_the_filled_run,
  text_whose_font_was_destroyed_draws_nothing, text_on_the_gpu_matches_the_software_oracle,
  a_shape_after_a_glyph_run_matches_the_software_oracle,
  eviction_frees_unpinned_glyphs_and_changes_no_pixel, a_rotated_text_run_draws_rotated,
  a_transformed_text_run_reaches_the_gpu_and_matches_the_oracle,
  two_hundred_presents_through_one_font_leave_the_glyph_cache_where_they_found_it,
  a_text_build_does_not_copy_the_font_file; local helpers: WALK_TEXT, WALK_SIZE, WALK_PENS,
  walk_scene, GPU_TEXT, GPU_TEXT_THEN_SHAPE, EVICTION, ROTATED_TEXT, UPRIGHT_TEXT,
  graphics_alloc_bytes, text_rebuild_alloc_bytes
- `tests/canvas/rt_canvas_font/face_index_and_kerning.rs` (644) — 11 tests:
  face_index_of_the_fixture_is_its_own_tables,
  face_index_of_a_font_with_no_head_has_no_units_per_em, face_index_answers_the_fixture_exactly,
  face_index_answers_a_format_4_cmap_exactly,
  face_index_answers_an_unsorted_overlapping_format_12_cmap_exactly,
  face_index_answers_real_system_fonts_exactly (`#[cfg(target_os = "macos")]`),
  a_load_destroy_cycle_keeps_only_the_fonts_own_blocks, kern_pairs_measure_at_the_kerned_width,
  face_kern_reads_each_fixtures_pairs,
  a_kerned_pair_draws_at_the_kerned_distance_filled_and_stroked,
  a_real_font_kerns_as_harfbuzz_does (`#[cfg(target_os = "macos")]`); local helpers:
  face_dumps, rename_table, LOAD_FIXTURE, Segment, cmap4, face_check, assert_check_clean,
  live_after_cycles, class_def2, class_def1, pair_pos2, extension, kern_table, X_PLACEMENT,
  Y_PLACEMENT, Y_ADVANCE, X_ADVANCE_DEVICE, gpos_format2_extension, kern_only,
  gpos_wide_value_records, kerning_fixtures, kerned_widths, av_right_edge
- `tests/canvas/rt_canvas_font/shaping.rs` (774) — 15 tests:
  measure_text_over_a_font_list_sums_each_font_at_its_own_scale,
  measure_text_over_a_one_font_list_is_the_single_font_form,
  a_two_font_string_draws_each_glyph_from_its_own_font,
  closing_the_first_font_draws_the_string_from_the_next,
  shape_run_marks_every_grapheme_boundary_and_scalar,
  letter_spacing_moves_the_next_character_by_the_spacing, a_spacing_only_change_redraws,
  measure_text_spacing_adds_between_graphemes_only,
  decoration_metrics_come_from_post_and_os2_or_the_defaults,
  caret_stops_agree_with_measuring_and_with_graphemes,
  caret_stops_place_kerned_characters_and_refuse_closed_fonts,
  liga_measures_ligatures_and_not_with_spacing, liga_draws_the_ligature_glyph,
  liga_carets_split_the_ligature, a_real_font_ligates_as_harfbuzz_does
  (`#[cfg(target_os = "macos")]`); local helpers: fallback_font, run_two_fonts,
  two_font_columns, RUN_STRIDE, RUN_SCALAR, RUN_GAP, shaped_runs, spaced_right_edge,
  decorated_font, decoration, GID_F, GID_I, GID_L, GID_FI, GID_FFI, ligature_subst,
  gdef_carets, ligature_font, gsub_liga, ligating_font, liga_columns

Surface: root declares 4 topics as `#[path = "rt_canvas_font/<name>.rs"] mod <name>;`; target name `rt_canvas_font` unchanged; no visibility change (`FontRun` moves with its only user; `truetype_fixture_with`'s nested `be16`/`be32` move with it).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_font.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | includes: none | the fixture builder stays in the root, so `scripts/remote-common.sh:47` and `scripts/test-canvas-vulkan.sh:88` stay valid | stale `scripts/man-run-examples.sh:260` `tests/rt_canvas_font.rs` → `tests/canvas/rt_canvas_font.rs` (the builder's real path; `ls tests/rt_canvas_font.rs` → No such file).

#### 4.6 `tests/canvas/rt_canvas_rasteriser.rs` — 4712 test LOC, 113 tests

- `tests/canvas/rt_canvas_rasteriser.rs` (575) — root: header (`mod common`, `use`s); helpers:
  WIDTH, HEIGHT, render, pixel, scene, RING_HELPERS, lit; tests kept (12), the map topics
  placement (4), path_placement (3) and group_affine (5) with their local helpers NO_TRANSFORM,
  WHITE_BOX, QUARTER; declares `shapes`, `strokes_and_dashes`, `clips_blends_and_transforms`,
  `ellipses_and_corners`, `gradients`, `groups`
- `tests/canvas/rt_canvas_rasteriser/shapes.rs` (699) — 19 tests: the map topics basic_shapes (7),
  cache_and_polygons (5) and fill_rules_and_square_caps (3), plus bug-1088's four butt-arc tests
  (a_magnified_butt_arc_draws_its_band_and_antialiased_ends_only,
  a_zero_or_tiny_sweep_butt_arc_draws_nothing_behind_its_centre,
  a_butt_arc_swept_a_whole_turn_is_the_whole_ring,
  a_butt_arc_whose_end_is_before_its_start_draws_nothing, L215–383 among the basic shapes);
  local helpers: PENTAGRAM
- `tests/canvas/rt_canvas_rasteriser/strokes_and_dashes.rs` (595) — 13 tests: the map topics
  joins (2), ellipse_dashes (2), dash_patterns (4) and caps (5); local helpers:
  ellipse_arc_length, ellipse_angle_at
- `tests/canvas/rt_canvas_rasteriser/clips_blends_and_transforms.rs` (632) — 17 tests: the map
  topics clips (5), path_clips (3), blend_and_transparency (3), transforms (5) and
  distance_correction_measurement (1, the `#[ignore]`d `measure_the_transformed_distance_correction`);
  local helpers: clip_ring
- `tests/canvas/rt_canvas_rasteriser/ellipses_and_corners.rs` (722) — 14 tests: the map topics
  ellipse_newton_measurement (1, the `#[ignore]`d `measure_the_ellipse_newton_iteration_count`,
  moved whole with its nested `truth`, `solve`, `bisect`, `steps`), ellipses (6),
  corner_rule_radii (3) and corner_rule_binding (4); local helpers: corner_reach,
  top_left_rounded
- `tests/canvas/rt_canvas_rasteriser/gradients.rs` (720) — 19 tests: the map topics gradients (6),
  gradient_strokes_and_order (4), gradient_transforms (4) and gradient_focal (5); local helpers:
  ramp, NO_T, focal, ramp_box, stroke_ramp
- `tests/canvas/rt_canvas_rasteriser/groups.rs` (787) — 19 tests: the map topics group_presents
  (4), group_rendering (6), group_retirement (4, including
  `scene_draws_shares_one_base_between_a_diamonds_two_draws`, the `draws=` assertions) and
  resource_lifetime_mid_frame (5); local helpers: render_stdout, stat

Surface: root declares 6 topics as `#[path = "rt_canvas_rasteriser/<name>.rs"] mod <name>;`; target name `rt_canvas_rasteriser` unchanged; no visibility change. Both `#[ignore]`d measurements stay runnable with `cargo test --test rt_canvas_rasteriser -- --ignored` (filter unchanged).
Sweeps: citations `06_canvas.md:266` `path_placement_a_clipped_polygon_moves_whole_with_its_position`, `:1536` `group_affine_a_nested_group_composes_with_its_parent` and `:1548` `group_affine_a_scaled_group_scales_its_strokes` stay in the root (K6) | includes: none | `planning/plan-175-B-canvas-draw-node-table.md:260` "`tests/canvas/rt_canvas_rasteriser.rs`'s `draws=` assertions" → "`tests/canvas/rt_canvas_rasteriser/groups.rs`'s `draws=` assertions" if plan-175-B is still under `planning/` (not `planning/completed/`) when Phase 4 lands; `planning/plan-175-C-canvas-group-node-instancing.md:147` names the target for new tests and stays valid.

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

Every phase follows the same order: K1 on the unsplit tree, then split each file per its §4 entry,
then the phase's sweeps, then K2–K5 (and K6 where the phase's roots hold a citation), then commit
the new files, the roots, the swept docs and the ceiling file together. All runs happen on the
macOS development host, which is the only host with Metal.

### Phase 1 — The GPU golden and Metal suites (§4.1–§4.2)

Two files with 56 scene includes to repoint, two citations that stay in their roots, the Vulkan
`sed` pin, and the stale `.ai/testing-gates.md` path.
`T="rt_canvas_golden rt_canvas_metal"`, `RE="rt_canvas_(golden|metal)"`.

- [x] K1 → 102 tests. Record `for c in GROUPS POLYLINES; do sed -n "/^const $c: &str = r#\"/,/^\"#;\$/p" tests/canvas/rt_canvas_golden.rs | wc -l; done` → `26`, `43`.
      Done: `bash /tmp/p176/dklist.sh before rt_canvas_golden rt_canvas_metal` (K1, `/tmp/176D-<t>.before`) → golden 28, metal 74, total 102; the two `sed` ranges → `26`, `43`.
- [x] `rt_canvas_golden.rs` → root (keeps 19 tests, `GROUPS`, `POLYLINES` unindented, the 4 scene includes and the render helpers) + `ellipses_gradients_polylines.rs` (§4.1).
      Done by name with `python3 -I /tmp/p176/dsplit.py /tmp/p176/D-golden.json "$PWD"` (root 19 tests, topic 9). Sizes (`census.py --file`): root `0 788`, topic `0 397`. Lossless: `python3 -I /tmp/p176/dverify.py <HEAD copy> <root> <topic>` → missing `[]`, extra `[]`, 2 decl lines.
- [x] `rt_canvas_metal.rs` → root (keeps 10 tests and every shared helper) + `scenes_over_black.rs`, `scenes_and_passes.rs` (§4.2). The 56 scene includes in the two topics become `"../scenes/…"`.
      Done with `dsplit.py /tmp/p176/D-metal.json` (root 10, `scenes_over_black` 33, `scenes_and_passes` 31). Sizes: root `0 730`, `scenes_over_black` `0 380`, `scenes_and_passes` `0 653`. Lossless: `dverify.py` → missing `[]`, extra `[]`, 4 decl lines, include rewrites 56.
- [x] Include sweep: `grep -rn 'include_str!("scenes/' tests/canvas/rt_canvas_metal tests/canvas/rt_canvas_metal.rs` → no output;
      `cat tests/canvas/rt_canvas_metal/*.rs | grep -c 'include_str!("../scenes/'` → `56`;
      `grep -c 'include_str!("scenes/' tests/canvas/rt_canvas_golden.rs` → `4`.
      Done: → no output; `56`; `4`.
- [x] `.ai/testing-gates.md:332`: `tests/rt_canvas_metal.rs` → `tests/canvas/rt_canvas_metal.rs`. This is a pre-existing stale path (evidence: `ls tests/rt_canvas_metal.rs` → No such file).
      Done: `ls tests/rt_canvas_metal.rs` → No such file or directory; line 332 repointed; `grep -n 'tests/rt_canvas' .ai/testing-gates.md` → no output.
- [x] Tighten: K5 and commit the ceiling file and `.ai/testing-gates.md`.
      Done: `bash /tmp/p176/dk45.sh 'rt_canvas_(golden|metal)'` → K4 no output; guard → `TIGHTEN test 788 (ceiling 1182)` golden and `TIGHTEN test 730 (ceiling 1758)` metal; `MFB_LOC_TIGHTEN=1` exit 0; foreign ceiling diffs none; own diffs `-ceiling test 1182 …golden.rs`, `-ceiling test 1758 …metal.rs`; re-run guard grep → no output.

Acceptance: 102 tests are listed unchanged and pass on Metal, both citations still resolve to
their symbols in the roots, and the Vulkan script still extracts its scenes.
  Check: K2 → no output; K3 → 2 × ok; K4 → no output; K5 → 2 TIGHTEN rows, then none; K6 → ok,
  and `grep -n 'fn the_gpu_draws_the_group_affine_scene_the_reference_shows' tests/canvas/rt_canvas_golden.rs`
  and `grep -n 'fn a_group_with_a_matrix_draws_on_metal_and_matches_the_software_oracle' tests/canvas/rt_canvas_metal.rs`
  → 1 hit each; the `sed` line counts are again `26`, `43`; `grep -n 'tests/rt_canvas' .ai/testing-gates.md` → no output (est. 15 min).
  Verified 2026-10-18: K2 `bash /tmp/p176/dklist.sh after rt_canvas_golden rt_canvas_metal` → no `DIFF`, 28 + 74 = 102;
  K3 `bash scripts/test-fast.sh --test rt_canvas_golden --test rt_canvas_metal` → `test-fast: ok`
  (`rt_canvas_golden` `ok. 28 passed; 0 failed`, `rt_canvas_metal` `ok. 74 passed; 0 failed`); K4 → no output;
  K5 → 2 TIGHTEN rows, then none; K6 `cargo test -p mfb --bin mfb spec_citations_resolve` → `ok. 1 passed`,
  the two `grep -n 'fn …'` → golden.rs:929, metal.rs:595 (1 hit each); `sed` counts → `26`, `43`;
  `grep -n 'tests/rt_canvas' .ai/testing-gates.md` → no output; `cargo fmt --all -- --check` → exit 0.
Commit: a9cdccae0

### Phase 2 — Curves and particles (§4.3–§4.4)

Two files with no citation and no include; every shared helper stays in its root.
`T="rt_canvas_curves rt_canvas_particles"`, `RE="rt_canvas_(curves|particles)"`.

- [x] K1 → 69 tests.
      Done: `bash /tmp/p176/dklist.sh before rt_canvas_curves rt_canvas_particles` → curves 40, particles 29, total 69.
- [x] `rt_canvas_curves.rs` → root (all helpers, both `impl Scene` blocks, no tests) + `reduction_and_pieces.rs`, `curve_shapes.rs`, `picture_curves.rs` (§4.3).
      Done with `dsplit.py /tmp/p176/D-curves.json` (root 0 tests; 18, 17, 5). Sizes: root `0 748`, `reduction_and_pieces` `0 772`, `curve_shapes` `0 678`, `picture_curves` `0 243`. Lossless: `dverify.py` → missing `[]`, extra `[]`, 6 decl lines.
- [x] `rt_canvas_particles.rs` → root (keeps 16 tests and the render helpers) + `lifecycle_and_model.rs`, with `MODEL_DRIVER` moved verbatim beside `model()` (§4.4).
      Done with `dsplit.py /tmp/p176/D-particles.json` (root 16, topic 13). Sizes: root `0 548`, topic `0 669`. Lossless: `dverify.py` → missing `[]`, extra `[]`, 2 decl lines.
- [x] Byte check of the moved driver string (250 lines, unindented, so the same `sed` range
      selects it in both files):
      `git show HEAD:tests/canvas/rt_canvas_particles.rs | sed -n '/^const MODEL_DRIVER: &str = r#"/,/^"#;$/p' | cmp - <(sed -n '/^const MODEL_DRIVER: &str = r#"/,/^"#;$/p' tests/canvas/rt_canvas_particles/lifecycle_and_model.rs)`
      → no output (`HEAD` is the tree before this phase; the range prints 250 lines today).
      Done (the session's shell refuses process substitution, so the HEAD side is a copy of the clean pre-split file, `/tmp/p176/D-orig-particles.rs`): the two `sed` ranges written to files, `cmp` → no output, exit 0; 250 lines.
- [x] (discovered) K3's landing tier failed 6 gate tests that share fixed `$TMPDIR/mfb-bug470-*` trees with any other
      checkout testing at the same time (another agent's `/tmp/b1014/tree` was running `gate_mutual_exclusion`, `pgrep -fl`).
      Fixed in `9755a52bc` (tree names carry a hash of `CARGO_MANIFEST_DIR`; no assertion changed):
      `cargo test --test gate_mutual_exclusion --test gate_release_on_normal_exit` → `ok. 5 passed`, `ok. 3 passed`.
- [x] Tighten: K5 and commit the ceiling file.
      Done: `bash /tmp/p176/dk45.sh 'rt_canvas_(curves|particles)'` → K4 no output; `TIGHTEN test 748 (ceiling 2432)` curves, `TIGHTEN test 548 (ceiling 1214)` particles; tighten exit 0; no foreign ceiling diff; own `-ceiling test 2432 …curves.rs`, `-ceiling test 1214 …particles.rs`; re-run → no output.

Acceptance: 69 tests are listed unchanged and pass, and the driver string is byte-identical.
  Check: K2 → no output; K3 → 2 × ok; K4 → no output; K5 → 2 TIGHTEN rows, then none; the `cmp`
  → no output (est. 12 min).
  Verified 2026-10-18: K2 `bash /tmp/p176/dklist.sh after rt_canvas_curves rt_canvas_particles` → no `DIFF`, 40 + 29 = 69;
  K3 `bash scripts/test-fast.sh --test rt_canvas_curves --test rt_canvas_particles` → `rt_canvas_curves` `ok. 40 passed; 0 failed`,
  `rt_canvas_particles` `ok. 29 passed; 0 failed`, fmt/check/build/skips ok; its `tests` step failed only in
  `gate_mutual_exclusion` (4) and `gate_release_on_normal_exit` (2), which collided with another checkout's concurrent run of
  the same binary (fixed in `9755a52bc`; `cargo test --test gate_mutual_exclusion --test gate_release_on_normal_exit` → ok 5, ok 3);
  K4 → no output; K5 → 2 TIGHTEN rows, then none; the `cmp` → no output.
Commit: 009ecbc32

### Phase 3 — rt_canvas_font (§4.5)

The re-mapped layout, with the most cross-part helpers, and the stale `man-run-examples.sh` path.
`T="rt_canvas_font"`, `RE="rt_canvas_(font)"`.

- [x] K1 → 60 tests.
      Done: `bash /tmp/p176/dklist.sh before rt_canvas_font` → 60.
- [x] Re-measure before cutting: `python3 -I tools/loc-census/census.py --file tests/canvas/rt_canvas_font.rs` → `0 3547`. If it differs, re-run the §2 part sums for the new items before splitting.
      Done: → `0 3547`.
- [x] `rt_canvas_font.rs` → root (the fixture builder, the render family and the table builders; no tests) + `loading_and_measure.rs`, `drawing.rs`, `face_index_and_kerning.rs`, `shaping.rs` (§4.5).
      Done with `dsplit.py /tmp/p176/D-font.json` (root 0 tests; 17, 17, 11, 15). Sizes: root `0 614`, `loading_and_measure` `0 752`, `drawing` `0 775`, `face_index_and_kerning` `0 644`, `shaping` `0 774`. Lossless: `dverify.py` → missing `[]`, extra `[]`, 8 decl lines.
- [x] `scripts/man-run-examples.sh:260`: `tests/rt_canvas_font.rs` → `tests/canvas/rt_canvas_font.rs`. Pre-existing stale path (`ls tests/rt_canvas_font.rs` → No such file).
      Done: `ls tests/rt_canvas_font.rs` → No such file or directory; line 260 repointed.
- [x] Reference sweep: `grep -rn 'tests/rt_canvas' scripts .ai AGENTS.md src/docs .github` → no output.
      Done: → no output (exit 1).
- [x] Tighten: K5 and commit the ceiling file and `scripts/man-run-examples.sh`.
      Done: `bash /tmp/p176/dk45.sh 'rt_canvas_(font)'` → K4 no output; `TIGHTEN test 614 (ceiling 3547)`; tighten exit 0; no foreign ceiling diff; own `-ceiling test 3547 …font.rs`; re-run → no output.

Acceptance: 60 tests are listed unchanged and pass (the four macOS-only ones included), and every
live reference to the fixture builder names the file that holds it.
  Check: K2 → no output; K3 → ok; K4 → no output; K5 → 1 TIGHTEN row, then none;
  `grep -n 'fn truetype_fixture_with' tests/canvas/rt_canvas_font.rs` → 1 hit (est. 12 min).
  Verified 2026-10-18: K2 `bash /tmp/p176/dklist.sh after rt_canvas_font` → no `DIFF`, 60;
  K3 `bash scripts/test-fast.sh --test rt_canvas_font` → `test-fast: ok` (`rt_canvas_font` `ok. 60 passed; 0 failed`,
  the four `#[cfg(target_os = "macos")]` tests included on this macOS host); K4 → no output; K5 → 1 TIGHTEN row, then none;
  `grep -n 'fn truetype_fixture_with' tests/canvas/rt_canvas_font.rs` → 1 hit (:101).
Commit: 77a35dc3b

### Phase 4 — rt_canvas_rasteriser (§4.6)

The largest split: 113 tests in 7 parts, and three citations that stay in the root.
`T="rt_canvas_rasteriser"`, `RE="rt_canvas_(rasteriser)"`.

- [x] K1 → 113 tests (including the two `#[ignore]`d measurements).
      Done: `git show 77a35dc3b:tests/canvas/rt_canvas_rasteriser.rs > /tmp/p176/before.rs; python3 -I /tmp/p176/names.py /tmp/p176/before.rs`
      (every `#[test]` fn name) → 113 names.
- [x] `rt_canvas_rasteriser.rs` → root (575; keeps the placement, path-placement and group-affine tests) + six topics (§4.6).
      Done: `census.py --file` → root 575; clips_blends_and_transforms 632, ellipses_and_corners 722, gradients 720,
      groups 787, shapes 699, strokes_and_dashes 595. Motion check `python3 -I /tmp/p176/motion.py <root> <dir> /tmp/p176/before.rs`
      (non-blank line multiset, skipping `//!`, `use super::*;` and the `#[path]`/`mod` pairs) → 0 lines only after; the 16
      lines only before are the root's own `//!` header and `mod common;` pair, both still at the top of the root.
- [x] Declaration check: `grep -c '^#\[path = "rt_canvas_rasteriser/' tests/canvas/rt_canvas_rasteriser.rs` → `6`;
      `ls tests/canvas/rt_canvas_rasteriser/*.rs | wc -l` → `6`.
      Done: → `6`, `6`.
- [x] `planning/plan-175-B-canvas-draw-node-table.md:260`: if the file is still under `planning/`
      Done: `ls planning/plan-175-B-canvas-draw-node-table.md` → exists; line 260 repointed to `…/groups.rs`
      (`grep -c 'draws=' tests/canvas/rt_canvas_rasteriser/groups.rs` → 5).
      (`ls planning/plan-175-B-canvas-draw-node-table.md` → exists), repoint
      "`tests/canvas/rt_canvas_rasteriser.rs`'s `draws=` assertions" to
      "`tests/canvas/rt_canvas_rasteriser/groups.rs`'s `draws=` assertions"; if it has moved to
      `planning/completed/`, mark this task moot with that `ls` as evidence.
- [x] Tighten: K5 and commit the ceiling file (and plan-175-B if edited).
      Done: guard → `TIGHTEN  test 575 (ceiling 4712)  tests/canvas/rt_canvas_rasteriser.rs`; `MFB_LOC_TIGHTEN=1` → own diff
      `-ceiling test 4712 tests/canvas/rt_canvas_rasteriser.rs` only; re-run → no output.

Acceptance: 113 tests are listed unchanged and pass, and the three citations still resolve to
their symbols in the root.
  Check: K2 → no output; K3 → ok; K4 → no output; K5 → 1 TIGHTEN row, then none; K6 → ok and
  `grep -nE 'fn (path_placement_a_clipped_polygon_moves_whole_with_its_position|group_affine_a_nested_group_composes_with_its_parent|group_affine_a_scaled_group_scales_its_strokes)\(' tests/canvas/rt_canvas_rasteriser.rs`
  → 3 hits; `grep -c 'draws=' tests/canvas/rt_canvas_rasteriser/groups.rs` → ≥ 1 (est. 15 min).
  Verified 2026-10-19: K2 `cargo test --test rt_canvas_rasteriser -- --list` leaf names `diff` the K1 list → none, 113;
  K3 `bash scripts/test-fast.sh --test rt_canvas_rasteriser` → `test-fast: ok` (`ok. 111 passed; 0 failed; 2 ignored`);
  K4 → no output; K5 → 1 TIGHTEN row, then none; K6 `cargo test -p mfb --bin mfb spec_citations_resolve` → `ok. 1 passed`;
  the 3-fn grep → 3 hits (:408, :616, :645); `draws=` in groups.rs → 5.
  Final gate (all of D): the guard grep over the six files → no output; the ceiling-file grep → no output; the
  `census.py --file … | awk '$2 > 900'` loop → no output; spec citation count `grep -cE … 06_canvas.md` → `5`;
  `test_targets_registered` ran inside K3's test-fast call → ok.
Commit: —

## Validation Plan

- Tests: none are added. Every moved test runs in its phase's K3. The two `#[ignore]`d
  rasteriser measurements are covered by K1/K2 (listed) and K3 (compiled); they move whole, so
  no run is needed.
- Coverage check: `tests/` is outside the coverage denominator (`scripts/coverage-common.sh`
  IGNORE), and no compiler or runtime code changes.
- Runtime proof: each phase's K3 on the macOS Metal host. The Linux Vulkan script's inputs (the
  `GROUPS`/`POLYLINES` `sed` extraction) are checked in Phase 1 without a Linux run, because the
  extracted text is unchanged.
- Doc sync: `.ai/testing-gates.md:332`, `scripts/man-run-examples.sh:260`, and (if still open)
  `planning/plan-175-B-canvas-draw-node-table.md:260`. No `mfb spec` text changes. Citation
  count check: `grep -cE '\[\[tests/canvas/rt_canvas_(rasteriser|golden|metal)\.rs:' src/docs/spec/app/06_canvas.md`
  → `5` before Phase 1 and after Phase 4.
- Final gate (once, after Phase 4):
  `cargo test --test file_size_budget 2>&1 | grep -E 'tests/canvas/rt_canvas_(curves|font|golden|metal|particles|rasteriser)(\.rs|/)'` → no output;
  `grep -E '^ceiling .*tests/canvas/rt_canvas_(curves|font|golden|metal|particles|rasteriser)' tests/guards/file_size_ceilings.txt` → no output;
  `for f in tests/canvas/rt_canvas_{curves,font,golden,metal,particles,rasteriser}.rs tests/canvas/rt_canvas_{curves,font,golden,metal,particles,rasteriser}/*.rs; do python3 -I tools/loc-census/census.py --file $f; done | awk '$2 > 900'` → no output;
  `cargo test -p mfb --bin mfb spec_citations_resolve` and `cargo test --test test_targets_registered` → ok
  (est. 4 min). The full workspace suite runs once, in plan-176-V.

## Decisions

- **Rule revised — owner, 2026-10-18** (replaces the 2026-10-14 "code ≤ 1000, test ≤ 250" rule).
  All counts are non-comment, non-blank LOC, over `.rs`/`.mfb` under `src/`, `packages/`,
  `examples/`, `repository/` and `tests/`:

  | Kind | Applies to | Limit | Over the limit → |
  |---|---|---|---|
  | `code` | code LOC of a file that is not a test-only file | 1000 | split the file into modules |
  | `inline` | inline test LOC (`#[cfg(test)]` items / MFBASIC `TESTING … END TESTING`) inside a file that is not a test-only file | 250 | move the tests out into their own test-only file(s) |
  | `test` | total LOC of a test-only file (path component `tests`, `test_*.mfb`, `#![cfg(test)]`, or a `#[cfg(test)] mod x;` target and its subtree) | 1000 | split the test-only file into modules |

  There are no exemptions; generated files (`GENERATED` banner) are skipped automatically. The
  guard warns (banner) and never fails. Ceiling rows are `ceiling <code|inline|test> <n> <path>`.
  A new module is always an acceptable split. Parts target ≤ 900 LOC. Every D file is kind
  `test`.
- Cross-part helpers stay in the root rather than moving into `pub(super)` support files —
  author, 2026-10-18. No visibility edit is needed, every root is ≤ 788, and the font fixture
  builder keeps the path two scripts cite.
- `MODEL_DRIVER` stays an inline raw string, and `measure_the_ellipse_newton_iteration_count`
  keeps its nested fns — author, 2026-10-18. Each fits whole in a part of ≤ 900.
- Parts merge neighbouring map topics; no new seams — author, 2026-10-18.
- Glob-cycle imports compile without warnings (`/tmp/loc/probe2` printed `6`, no warning) — verified.
- Pre-existing stale references found while sweeping are fixed in the phase that touches the file:
  `.ai/testing-gates.md:332` and `scripts/man-run-examples.sh:260` — AGENTS.md "never leave a bug".
- Live docs (scripts, open plans, open bugs) are repointed when a symbol they name moves. Archived
  docs under `*/completed/` are not edited — author, 2026-10-14.

## Corrections

- 2026-10-18 — Rule revised by the owner: test-only files limit 1000 (was 250), inline tests 250,
  no exemptions. Dropped 12 files, each now ≤ 1000 test LOC (`python3 -I /tmp/loc/census.py "$PWD"`
  at `ce7a995ca`): `rt_canvas_autopaint`, `rt_canvas_damage`, `rt_canvas_frame_stats`,
  `rt_canvas_graphics_thread`, `rt_canvas_group_ownership`, `rt_canvas_hit`,
  `rt_canvas_image_decode`, `rt_canvas_picture`, `rt_canvas_picture_fit`,
  `rt_canvas_present_deep_copy`, `rt_canvas_present_leak`, `rt_canvas_tile` (all
  `tests/canvas/*.rs`). Other structural changes: the 6 kept files are re-laid out as 17 new
  `.rs` files (was 113 new files, 112 `.rs` + 1 `.mfb`, across 18 targets); support files are
  folded into the roots, so no `pub(super)` edits remain; the `model_driver.mfb` extraction and
  the Newton nested-fn hoist are removed (neither unit is over a limit any more); no spec
  citation moves (the five kept citations' symbols stay in their roots; the damage citation left
  with its file); the font fixture builder stays in its root, so the script/plan/bug repointing
  for it is gone, leaving only the stale `man-run-examples.sh` path fix; phases are 4 (was 5).
  Re-measured at `ce7a995ca`: `rt_canvas_font` 1763 → 3547 LOC and 33 → 60 tests (plan-162,
  bug-1091; re-mapped), `rt_canvas_metal` 1710 → 1758 LOC and 71 → 74 tests (+3 scene includes,
  56 total), `rt_canvas_rasteriser` 4575 → 4712 LOC and 109 → 113 tests (bug-1088); spec
  citation lines moved to :1536, :1548, :1628, :1629 and the stale testing-gates path to :332.
  plan-175-B:260 sweep added.
- 2026-10-18 (execution) — the plan's `/tmp/loc/census.py`, `/tmp/loc/rw-D/items.py` and
  `assign3.py` are replaced by `python3 -I tools/loc-census/census.py --file PATH` (plan-176-A) and
  two scratch tools: `/tmp/p176/dsplit.py` moves top-level items by name (it lexes strings, raw
  strings and comments, so a brace inside a scene string never ends an item; it asserts each
  part's and the root's test count) and `/tmp/p176/dverify.py` checks the non-blank lines of the
  pre-split file == root + parts minus headers, `use super::*;` and the `#[path]`/`mod` pairs.
  All six files matched their §2 measurement at HEAD `d4c18d5ea`, so §4's item lists were used
  unchanged; every part measures exactly its §4 number except the `rt_canvas_metal` root, 730
  (§4.2 says 731; the census is the truth, and both are under 900).
- 2026-10-18 (execution) — owner instruction 2026-10-18: landing tier per phase; the full suite
  runs once before the plan merges. K3 runs as `bash scripts/test-fast.sh --test <t>...` (fmt,
  `cargo check --workspace --all-targets`, unit tests, the guard binaries and the named targets),
  and the final gate's `cargo test --test test_targets_registered` runs inside that call.
- 2026-10-18 (execution) — part files start with a comment-only `//!` header naming what they
  hold and where the shared helpers live, then `use super::*;` (B/C's convention). The header is
  comment-only, so it adds no LOC.
- 2026-10-18 (execution) — Phase 2's landing tier exposed a pre-existing cross-checkout collision
  in `tests/gate/gate_mutual_exclusion.rs` and `tests/gate/gate_release_on_normal_exit.rs` (fixed
  `$TMPDIR` tree names); fixed in its own commit `9755a52bc` and recorded as a Phase 2 task.
- 2026-10-19 (execution) — the Phase 4 split was left uncommitted by an interrupted session, and
  `/tmp/p176/` (`dklist.sh`, `dsplit.py`, `dverify.py`) was gone. Phase 4 was re-verified from
  scratch against `77a35dc3b`: K1 is a static `#[test]` fn name list of the pre-split file
  (`/tmp/p176/names.py`), K2 diffs the compiled `--list` leaf names against it, and the motion
  check is `/tmp/p176/motion.py` (the non-blank line multiset of root + parts, minus headers and
  declarations, equals the pre-split file's).

## Summary

The risk is a topic that is never compiled (caught by K2's name list) and a scene include that
misses (a compile error, so it is loud). The spec citation check is manual, because the resolver
is file-level; K6's symbol grep makes it checkable, and with every cited symbol staying in its
root there is nothing to repoint. Nothing outside `tests/canvas/rt_canvas_{curves,font,golden,metal,particles,rasteriser}*`,
the ceiling file, one `.ai` line, one script comment line and (if still open) one line of
plan-175-B changes.

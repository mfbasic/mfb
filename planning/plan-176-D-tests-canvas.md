# plan-176-D: Split tests/canvas (18 integration-test files)

Last updated: 2026-10-14
Effort: large (3h–1d)
Depends on: plan-176-C

This sub-plan splits the 18 over-budget integration-test files under `tests/canvas/` (the list is
`/tmp/loc/groups.json["D"]`). Together they hold 19,120 test LOC and 468 `#[test]` fns. Each root
stays the `[[test]]` target and gains topic and support modules under `tests/canvas/<name>/`,
declared with `#[path]`, the layout plan-176-A verified. The edits beyond pure motion are all
decided ones:

- `pub(super)` on support items;
- 58 `include_str!("scenes/…")` paths that gain a `../`;
- one natural extraction: `rt_canvas_particles`' 250-LOC `MODEL_DRIVER` raw string moves to
  `include_str!("model_driver.mfb")`;
- hoisting the four fns nested inside `rt_canvas_rasteriser`'s 259-LOC
  `measure_the_ellipse_newton_iteration_count` into a sibling module;
- repointing six spec citations whose symbol moves;
- repointing the prose references to the font fixture builder;
- fixing two pre-existing stale `tests/rt_canvas_*.rs` paths.

**Single outcome:** after D lands, `cargo test --test file_size_budget` prints no row for any path
under `tests/canvas/` that this letter owns, and `tests/guards/file_size_ceilings.txt` has no
`ceiling` row for them. D adds no `exempt` row.

References:

- `planning/plan-176-A-file-size-guard.md` (rule, guard, ceiling format, `#[path]` layout) and
  `planning/plan-176-B-tests-runtime-a.md` (the gate kit K1–K5, restated in §3).
- `.ai/build-tooling.md` § "Splitting an over-budget file (plan-176)".
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations". This letter has six spec
  citations. `spec_citations_resolve` checks citations at file level only, so a moved symbol whose
  root file still exists passes silently, and the sweep below is manual. The file also has a stale
  path at :314, fixed in Phase 3.
- `.ai/canvas-threading.md` (:466, :478, :493, :496, :913, :976, :1020, :1093 name D root paths or
  targets; all stay valid).
- `.ai/specifications.md` (citation form `[[path:symbol]]`).
- Spec topic: `src/docs/spec/app/06_canvas.md`, citations at :266, :1500, :1512, :1548, :1592, :1593.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-C is complete and archived | `ls planning/plan-176-C-* planning/completed/plan-176-C-*` → only `planning/completed/plan-176-C-tests-runtime-b.md` | NOT MET (2026-10-14) |
| The guard and the oracle exist (plan-176-A) | `cargo test --test file_size_budget` → passes; `python3 -I tools/loc-census/census.py --file tests/canvas/rt_canvas_rasteriser.rs` → `0 4575` | NOT MET (2026-10-14; lands with A) |
| This letter's files have no uncommitted changes (bug-1092's edits to the ownership test and the canvas spec are committed first) | `git status --porcelain -- tests/canvas src/docs/spec/app/06_canvas.md scripts/remote-common.sh scripts/test-canvas-vulkan.sh scripts/man-run-examples.sh planning/completed/plan-162-B-kerning.md bugs/bug-1091-every-text-geometry-build-copies-the-whole-font-file.md .ai/testing-gates.md tests/guards/file_size_ceilings.txt` → empty | NOT MET (2026-10-14: ` M src/docs/spec/app/06_canvas.md`, ` M tests/canvas/rt_canvas_group_ownership.rs`) |
| rt_canvas_group_ownership still measures as mapped once committed | `python3 -I tools/loc-census/census.py --file tests/canvas/rt_canvas_group_ownership.rs` → `0 589`; `grep -cE '^\s*#\[test' tests/canvas/rt_canvas_group_ownership.rs` → `15` | MET for the on-disk file (2026-10-14: `/tmp/loc/census.py` → 589, grep → 15) |

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

- Every file of this letter, and every file it creates (including `model_driver.mfb`), is ≤ 250
  test LOC by `tools/loc-census/census.py --file`.
- For every one of the 18 targets, the sorted list of test names from `-- --list`, with module
  prefixes stripped, is identical before and after. Every target passes `cargo test --test <t>`
  on the macOS development host, which is the Metal host.
- Every `[[tests/canvas/…:symbol]]` citation names the file that holds the symbol, and every prose
  reference to the font fixture builder names `tests/canvas/rt_canvas_font/fixture.rs`.

### Non-goals (explicit constraints)

- No behavior change: no test body, assertion, scene source, golden PNG or `#[ignore]` reason
  changes. `MODEL_DRIVER`'s string value is byte-identical (Phase 4 check).
- No `[[test]]` target added, renamed or re-pathed. `Cargo.toml` is not edited.
- No test fn renamed. `measure_the_ellipse_newton_iteration_count` keeps its name,
  `#[ignore]` and body semantics, and only its nested fns move out.
- `tests/canvas/scenes/*.mfb` are not touched. The `GROUPS` and `POLYLINES` consts stay in the
  `rt_canvas_golden` root, unindented, because `scripts/test-canvas-vulkan.sh` extracts them
  with `sed`.

## 2. Current State

### Files and measured sizes

Test LOC are from the planning oracle on the working tree of `17a9b5799` (files read as on disk;
`python3 -I /tmp/loc/census.py "$PWD"`; after A it is `tools/loc-census/census.py --file <path>`).
Test counts are `grep -cE '^\s*#\[test' <file>`.

| File | Test LOC | Tests | New files | Largest part after |
|---|---|---|---|---|
| `tests/canvas/rt_canvas_autopaint.rs` | 828 | 23 | 5 | 201 |
| `tests/canvas/rt_canvas_curves.rs` | 2432 | 40 | 15 | 204 |
| `tests/canvas/rt_canvas_damage.rs` | 778 | 11 | 5 | 199 |
| `tests/canvas/rt_canvas_font.rs` | 1763 | 33 | 11 | 215 |
| `tests/canvas/rt_canvas_frame_stats.rs` | 304 | 13 | 1 | 190 |
| `tests/canvas/rt_canvas_golden.rs` | 1182 | 28 | 8 | 193 |
| `tests/canvas/rt_canvas_graphics_thread.rs` | 619 | 20 | 4 | 162 |
| `tests/canvas/rt_canvas_group_ownership.rs` | 589 | 15 | 3 | 205 |
| `tests/canvas/rt_canvas_hit.rs` | 526 | 14 | 3 | 161 |
| `tests/canvas/rt_canvas_image_decode.rs` | 722 | 17 | 4 | 199 |
| `tests/canvas/rt_canvas_metal.rs` | 1710 | 71 | 9 | 217 |
| `tests/canvas/rt_canvas_particles.rs` | 1214 | 29 | 7 (one `.mfb`) | 216 |
| `tests/canvas/rt_canvas_picture.rs` | 666 | 16 | 4 | 190 |
| `tests/canvas/rt_canvas_picture_fit.rs` | 313 | 9 | 1 | 180 |
| `tests/canvas/rt_canvas_present_deep_copy.rs` | 344 | 9 | 2 | 164 |
| `tests/canvas/rt_canvas_present_leak.rs` | 253 | 6 | 1 | 198 |
| `tests/canvas/rt_canvas_rasteriser.rs` | 4575 | 109 | 28 | 225 |
| `tests/canvas/rt_canvas_tile.rs` | 302 | 5 | 1 | 173 |
| **Total (18 files)** | **19,120** | **468** | **112 `.rs` + 1 `.mfb`** | |

Part sizes are the `span.py` sums of the moved items' non-comment lines, plus the added
lines (§3).

### Verified properties

- **Layout, globs and warnings.** These are the same facts as plan-176-B §2, from the same probes:
  the plan-176-A `#[path]` probe; `/tmp/loc_bcd_probe` (topic sees root imports, bare `common`
  helpers and support globs with no warning); `/tmp/loc/probe2` (glob cycles clean, printed `6`).
  `use super::*;` in a file that references no parent item warns.
- **Two D support files reference no parent item** and so omit `use super::*;`.
  `rt_canvas_font/fixture.rs` holds `minimal_truetype`, `truetype_fixture` and
  `truetype_fixture_with`, whose only free names are its own fns and their nested
  `be16`/`be32` (`python3 -I /tmp/loc_bcd_probe/s/idents.py tests/canvas/rt_canvas_font.rs 247-340`).
  `rt_canvas_rasteriser/ellipse_newton_solvers.rs` references only `std::f64::consts::FRAC_PI_2`
  by full path, `f64::INFINITY` and a nested `const COARSE` (`idents.py … 2583-2712`). Every other
  D support file calls a root or `common` helper (for example, `rt_canvas_image_decode`'s
  `png_builder` uses the root's `ihdr`).
- **Every D test compiles on the macOS development host.** The only crate-level gates are
  `#![cfg(unix)]` (`rt_canvas_hit.rs:16`, `rt_canvas_present_leak.rs:50`;
  `grep -n '^#!\[cfg' tests/canvas/rt_canvas_*.rs`), and these stay in the roots and cover their
  children. No D file gates an item to Linux or Windows. Metal tests need the macOS host, and that
  is the host the gates run on.
- **Scene includes are relative to the including file.** `grep -c 'include_str!("scenes/'` →
  `rt_canvas_golden.rs` 4, `rt_canvas_metal.rs` 53, `rt_canvas_graphics_thread.rs` 1. Each moves
  one directory deeper and becomes `"../scenes/…"`. No other D file includes by path.
- **`scripts/test-canvas-vulkan.sh` extracts two scenes from the golden root.** At :541 and :682,
  `sed -n '/^const GROUPS: &str = r#"/,/^"#;$/p' tests/canvas/rt_canvas_golden.rs` (and the same
  for `POLYLINES`) print 26 and 43 lines today. Both consts stay in the root, unindented.
- **`MODEL_DRIVER` is one raw string.** `const MODEL_DRIVER: &str = r#"` at line 256 closes with
  `"#;` at line 505, and its only use is the `{MODEL_DRIVER}` capture in `model()`'s `format!` at
  line 512 (`grep -n MODEL_DRIVER tests/canvas/rt_canvas_particles.rs`). An `include_str!` const is
  the same `&'static str` there.
- **The six spec citations** (`grep -rnoE '\[\[tests/canvas/[a-z_0-9]+\.rs[^]]*\]\]' src/docs .ai`)
  are exactly `06_canvas.md:266, 1500, 1512` (rasteriser), `:1548` (damage), `:1592` (golden) and
  `:1593` (metal). No `mfb man` page names a D file
  (`grep -rn 'tests/canvas/rt_' src/docs/man` → no hit).
- **Two references are stale today.** `.ai/testing-gates.md:314` names `tests/rt_canvas_metal.rs`,
  and `scripts/man-run-examples.sh:260` names `tests/rt_canvas_font.rs`. `ls` of both → `No such file or directory`.
  These are the only `tests/rt_canvas_*.rs` references in `.ai scripts AGENTS.md src/docs .github`.
- No D file respawns itself, reads its own source, or uses `module_path!`/`file!()`
  (`grep -nE 'current_exe|--exact|module_path!|file!\(\)'` → none). No D path is in
  `scripts/coverage-exceptions.txt` or a guard allowlist. No `tests/canvas/<name>/` directory
  exists yet. The existing `tests/canvas/scenes/` holds no D stem.

## 3. Design Overview

**Conventions (every file)** are plan-176-B §3's conventions:

- The root stays the `[[test]]` target. It keeps `#[path = "../common/mod.rs"] mod common;`, its
  `use` lines, its `#![cfg]` and the shared helpers listed in §4. Each child is declared as
  `#[path = "<name>/<child>.rs"] mod <child>;` (2 LOC each, already counted in §4's root numbers).
- Topics start with `use super::*;`, keep their local helpers private, and change no visibility.
- Support files get `pub(super)` on the items, fields and inherent methods §4 counts. The root adds
  `use <support>::*;` for each one. The two files marked "no `use super::*;`" omit it (§2).
- Code moves verbatim with doc comments and attributes. Item and test names are authoritative if
  lines drift from `17a9b5799`.

**Gate class: test-motion,** using plan-176-B's kit with `/tmp/176D-` prefixes and `tests/canvas/`
paths:

- **K1 list before**:
  `for t in $T; do cargo test -q --test $t -- --list 2>/dev/null | grep ': test$' | sed -E 's/^([a-z0-9_]+::)+//' | sort > /tmp/176D-$t.list; done; cat $(for t in $T; do echo /tmp/176D-$t.list; done) | wc -l` → the phase total.
- **K2 list after**:
  `for t in $T; do cargo test -q --test $t -- --list 2>/dev/null | grep ': test$' | sed -E 's/^([a-z0-9_]+::)+//' | sort | diff -q /tmp/176D-$t.list - >/dev/null || echo "DIFF $t"; done` → no output.
- **K3 run**: `cargo test $(for t in $T; do printf -- '--test %s ' $t; done)` → each `test result: ok`, 0 failed.
- **K4 warnings**: `cargo check --all-targets 2>&1 | grep -E -- "--> tests/canvas/$RE(\.rs|/)"` → no output.
- **K5 budget**: `cargo test --test file_size_budget 2>&1 | grep -E "tests/canvas/$RE(\.rs|/)"` → only TIGHTEN rows
  (one per root). Then `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`. Then
  `git diff -U0 tests/guards/file_size_ceilings.txt | grep -E '^[-+](ceiling|exempt)' | grep -vE "tests/canvas/$RE(\.rs|/)"` → no output.
  Re-running the first grep → no output.
- **K6 citations** (phases that edit `06_canvas.md`): `cargo test -p mfb --bin mfb spec_citations_resolve` → ok.
  Also, for each edited citation, `grep -n 'fn <symbol>' <new path>` → one hit, because the
  resolver itself does not check symbols.

**Where the risk concentrates.** rt_canvas_rasteriser has 109 tests in 27 topics. It is the
largest split and carries a decomposition of one test fn, so it goes last. The particles extraction
is the one place a string value could change, through a stray or missing trailing newline in the
`.mfb`. A byte compare against the pre-phase source closes that. The two GPU suites (golden, metal)
carry 57 scene includes and the Vulkan script's `sed` pin. A wrong include path is a compile error.
A moved `GROUPS`/`POLYLINES` would break only the Linux script, so Phase 3 checks the `sed` output.

**Rejected alternatives.**

- *Exempting `rt_canvas_particles/model.rs` (395 with the inline const)*. A natural extraction
  exists, so it is done (§Decisions).
- *Exempting `ellipse_newton_measurement.rs` (260)*. Hoisting nested fns is pure motion, so it is done (§Decisions).
- *Moving the scene strings of `rt_canvas_metal` into `tests/canvas/scenes/`*. That is not
  needed for size, and the GPU-rows scripts compile every `scenes/*.mfb`, so a new file there
  becomes a new script case.
- Helper promotion into `tests/common/`, per-topic `[[test]]` targets, and moving the root are
  rejected for the reasons in plan-176-B §3.

## 4. Detailed Design

### 4.0 Sweeps common to every file

- Spec citations: the six listed in §2, each repointed in the phase that moves its symbol, with K6.
- Prose references to D root paths in `.ai/canvas-threading.md`, `.ai/testing-gates.md` (:239,
  :292), `scripts/test-canvas-gpu-rows.sh:7`, `scripts/test-canvas-vulkan.sh:4,258,460` stay
  valid, because roots do not move. The exceptions are the font fixture builder (§4.15) and the
  two stale paths (Phases 3 and 4).
- Archived documents under `bugs/completed/` and `planning/completed/` record history and are not
  edited, for example `bugs/completed/bug-757-composite-glyphs-draw-nothing.md`, which cites
  `rt_canvas_font.rs:truetype_fixture`.
- Each entry gives the layout (path, LOC, contents), then "Surface", then "Sweeps".

#### 4.1 `tests/canvas/rt_canvas_frame_stats.rs` — 304 test LOC, 13 tests

- `tests/canvas/rt_canvas_frame_stats.rs` (190) — root: header (`mod common`, `use`s); helpers: run, run_with_stats, program, fields, field, number; tests kept (7): before_any_present_every_field_is_zero, identical_presents_count_as_presents_not_frames, identical_presents_are_skipped_whatever_the_id_length, dt_is_the_gap_between_the_last_two_presents, rates_fall_to_zero_after_a_second_idle, pps_above_the_ring_is_measured_not_capped, frame_seconds_and_renderer_after_a_drawn_frame; declares `wait_frame`
- `tests/canvas/rt_canvas_frame_stats/wait_frame.rs` (117) — 6 tests: wait_frame_paces_a_changing_scene, wait_frame_paces_an_unchanged_scene, wait_frame_before_any_present_returns, wait_frame_waits_for_pickup, wait_frame_survives_a_resize, a_malformed_display_override_is_ignored; local helpers: timed, loop_ms
Surface: root declares 1 topic + 0 support module(s) as `#[path = "rt_canvas_frame_stats/<name>.rs"] mod <name>;`; target name `rt_canvas_frame_stats` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_frame_stats.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | refs: none beyond the [[test]] stanza.

#### 4.2 `tests/canvas/rt_canvas_picture_fit.rs` — 313 test LOC, 9 tests

- `tests/canvas/rt_canvas_picture_fit.rs` (137) — root: header (`mod common`, `use`s); helpers: area, assert_area, assert_near, assert_rect; tests kept (9): canvas_picture_fit_stretch_fills_the_box, canvas_picture_fit_contain_fits_inside_and_centres, canvas_picture_fit_cover_covers_and_centres, canvas_picture_fit_a_circle_outline_is_within_a_twentieth_of_a_pixel, canvas_picture_fit_two_squares_intersect_in_their_overlap, canvas_picture_fit_a_square_inside_a_circle_is_the_square, canvas_picture_fit_each_ring_set_keeps_its_own_fill_rule, canvas_picture_fit_a_bar_across_a_hole_is_two_pieces, canvas_picture_fit_no_edge_is_interior_and_runs_are_joined; declares `driver`
- `tests/canvas/rt_canvas_picture_fit/driver.rs` (180) — shared helpers (`pub(super)` on 4 items): chunk, DRIVER, cases, value
Surface: root declares 0 topic + 1 support module(s) as `#[path = "rt_canvas_picture_fit/<name>.rs"] mod <name>;`; target name `rt_canvas_picture_fit` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`). Support files: their items become `pub(super)` (counts above); root adds `use <support>::*;` per support file (1).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_picture_fit.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | minimal split: root keeps all 9 tests; only the MFBASIC driver harness moves. `DRIVER` reads nothing by path.

#### 4.3 `tests/canvas/rt_canvas_present_deep_copy.rs` — 344 test LOC, 9 tests

- `tests/canvas/rt_canvas_present_deep_copy.rs` (164) — root: header (`mod common`, `use`s); helpers: app_ncode, ncode, function, instructions, position, calls, PUBLISH, SOURCE, label_at; tests kept (2): present_allocates_a_copy_of_the_scene, published_polygon_rings_are_copied_not_aliased; declares `publish_order`, `groups`
- `tests/canvas/rt_canvas_present_deep_copy/publish_order.rs` (81) — 3 tests: the_revision_is_published_after_the_items_and_count, an_identical_re_present_skips_the_publish, the_mode_gate_precedes_the_allocation; local helpers: scene_stores, PUBLISHED_OFFSETS
- `tests/canvas/rt_canvas_present_deep_copy/groups.rs` (105) — 4 tests: set_group_copies_both_the_items_and_the_name, a_group_slot_is_published_name_last, remove_group_clears_the_name_first_and_retires_the_buffer, the_group_table_is_absent_from_a_program_that_does_not_use_canvas; local helpers: SET_GROUP, GROUP_SOURCE
Surface: root declares 2 topic + 0 support module(s) as `#[path = "rt_canvas_present_deep_copy/<name>.rs"] mod <name>;`; target name `rt_canvas_present_deep_copy` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_present_deep_copy.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | refs: none beyond the [[test]] stanza.

#### 4.4 `tests/canvas/rt_canvas_present_leak.rs` — 253 test LOC, 6 tests

- `tests/canvas/rt_canvas_present_leak.rs` (198) — root: header (`mod common`, `use`s); helpers: FEW, MANY, FRAME_MS, ITEMS, FIXED_SLACK, TIGHT, LOOSE, live_bytes, assert_flat, unchanged, rebuilt_unchanged, changing, changing_layers; tests kept (4): re_presenting_an_unchanged_scene_holds_steady_memory, re_presenting_a_rebuilt_unchanged_scene_holds_steady_memory, presenting_a_changing_scene_holds_steady_memory, presenting_changing_layers_holds_steady_memory; declares `group_animation`
- `tests/canvas/rt_canvas_present_leak/group_animation.rs` (58) — 2 tests: animating_a_group_under_an_unchanged_scene_holds_steady_memory, animating_a_group_under_a_changing_scene_holds_steady_memory; local helpers: SCENE_ITEMS, group_animated
Surface: root declares 1 topic + 0 support module(s) as `#[path = "rt_canvas_present_leak/<name>.rs"] mod <name>;`; target name `rt_canvas_present_leak` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_present_leak.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | `#![cfg(unix)]` stays root.

#### 4.5 `tests/canvas/rt_canvas_tile.rs` — 302 test LOC, 5 tests

- `tests/canvas/rt_canvas_tile.rs` (173) — root: header (`mod common`, `use`s); helpers: WIDTH, HEIGHT, BLACK, pixels, source, tile, render, render_with, pixel; tests kept (2): atlas_regions_scale_and_out_of_bounds_samples_are_exact, closed_image_and_nonpositive_source_draw_nothing; declares `viewport_and_repaint`
- `tests/canvas/rt_canvas_tile/viewport_and_repaint.rs` (132) — 3 tests: flat_items_past_each_viewport_edge_publish_no_draw, viewport_culling_uses_composed_group_bounds_and_keeps_the_scene, set_bytes_on_a_presented_tile_image_repaints; local helpers: field, draw_runs, red_image, group
Surface: root declares 1 topic + 0 support module(s) as `#[path = "rt_canvas_tile/<name>.rs"] mod <name>;`; target name `rt_canvas_tile` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_tile.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | `Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/canvas/tile.png")` is absolute (unaffected).

#### 4.6 `tests/canvas/rt_canvas_hit.rs` — 526 test LOC, 14 tests

- `tests/canvas/rt_canvas_hit.rs` (125) — root: header (`mod common`, `use`s); helpers: FIXTURE_TTF, PRELUDE, run, press, over, assert_over, points; declares `hit_region`, `hit_at_and_id_path`, `over_id_and_events`
- `tests/canvas/rt_canvas_hit/hit_region.rs` (161) — 5 tests: hit_region_a_closed_shape_is_its_interior_and_its_stroke, hit_region_an_open_shape_is_its_stroke_with_three_pixels_of_slack, hit_region_text_is_its_layout_box, hit_region_follows_the_clip_and_the_transform, hit_region_a_particle_system_is_never_hit
- `tests/canvas/rt_canvas_hit/hit_at_and_id_path.rs` (135) — 5 tests: hit_at_the_upper_of_two_overlapping_items_answers, hit_at_an_item_in_a_transformed_group_is_at_its_transformed_place, hit_at_an_item_in_a_layer_above_wins, id_path_names_every_group_that_holds_the_item, id_path_a_picture_goes_in_front_of_its_shape; local helpers: picture
- `tests/canvas/rt_canvas_hit/over_id_and_events.rs` (114) — 4 tests: over_id_follows_the_scene_last_presented, over_id_is_resolved_when_the_only_id_is_on_or_inside_a_group, over_id_stays_right_across_many_changed_scenes, a_polled_event_is_a_whole_record
Surface: root declares 3 topic + 0 support module(s) as `#[path = "rt_canvas_hit/<name>.rs"] mod <name>;`; target name `rt_canvas_hit` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_hit.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | `#![cfg(unix)]` stays in the root (crate-level, covers topics). Archived bug docs cite `rt_canvas_hit.rs:FIXTURE_TTF` → stays root.

#### 4.7 `tests/canvas/rt_canvas_picture.rs` — 666 test LOC, 16 tests

- `tests/canvas/rt_canvas_picture.rs` (79) — root: header (`mod common`, `use`s); helpers: WIDTH, HEIGHT, BLACK, GREEN, render, pixel, scene, pixels, shaped, RED; declares `drawing`, `set_bytes`, `shaped`, `clip_group_and_destroy`
- `tests/canvas/rt_canvas_picture/drawing.rs` (162) — 4 tests: a_picture_draws_its_image_into_its_rectangle, scaling_samples_the_nearest_texel, the_fill_tints_the_image, the_fill_is_a_tint_and_transparency_is_the_opacity
- `tests/canvas/rt_canvas_picture/set_bytes.rs` (143) — 5 tests: set_bytes_on_a_presented_image_repaints, set_bytes_repaints_under_damage_tracking, set_bytes_on_an_undrawn_image_does_not_repaint, set_bytes_twice_between_frames_shows_the_last, set_bytes_repaints_a_shaped_picture
- `tests/canvas/rt_canvas_picture/shaped.rs` (190) — 4 tests: a_shaped_picture_is_its_image_inside_the_shape_and_its_stroke_on_top, contain_fits_inside_the_shape_and_cover_is_cut_to_it, clip_only_keeps_the_image_upright_and_clip_and_transform_turns_it, a_shaped_picture_with_a_clip_shows_only_where_both_cover; local helpers: BLUE
- `tests/canvas/rt_canvas_picture/clip_group_and_destroy.rs` (104) — 3 tests: clip_blend_and_transform_apply_to_a_picture, a_picture_in_a_group_moves_with_it, a_destroyed_image_draws_nothing
Surface: root declares 4 topic + 0 support module(s) as `#[path = "rt_canvas_picture/<name>.rs"] mod <name>;`; target name `rt_canvas_picture` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_picture.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | prose ref .ai/canvas-threading.md:496 (path only).

#### 4.8 `tests/canvas/rt_canvas_autopaint.rs` — 828 test LOC, 23 tests

- `tests/canvas/rt_canvas_autopaint.rs` (117) — root: header (`mod common`, `use`s); helpers: Built, Ran, WIDTH, field, build, run, read_or_empty; declares `bare_shape_and_cost`, `curves`, `redraw_and_pictures`, `states`, `state_scene`
- `tests/canvas/rt_canvas_autopaint/state_scene.rs` (112) — shared helpers (`pub(super)` on 16 items): BASE, ENTER_RGB, EXIT_RGB, HOVER_RGB, PRESSED_RGB, HOVER2_RGB, BUTTON, ENTER, EXIT, HOVER, PRESSED, ALL, STATES, states, scene, LONG_WINDOW
- `tests/canvas/rt_canvas_autopaint/bare_shape_and_cost.rs` (201) — 4 tests: an_autopaint_with_no_pointer_draws_exactly_its_bare_shape, an_autopaint_inside_a_group_draws_exactly_its_bare_shape, presenting_and_dropping_autopaints_holds_steady_memory, an_autopaint_frame_costs_linear_in_its_instances; local helpers: SHAPES, HEADER, program, live_bytes, MANY, graphics_scrubbed_bytes, walk_ms_per_frame
- `tests/canvas/rt_canvas_autopaint/curves.rs` (117) — 4 tests: an_autopaint_over_a_curve_polygon_follows_the_curve_not_its_box, an_autopaint_over_a_poly_curve_follows_its_stroke, a_curve_shapes_hover_stroke_does_not_widen_its_hit_region, a_picture_on_an_autopaint_over_a_curve_polygon_tints_on_hover; local helpers: CURVES, curves, curve_scene
- `tests/canvas/rt_canvas_autopaint/redraw_and_pictures.rs` (197) — 7 tests: state_survives_a_re_present_of_a_moved_shape, a_scene_with_no_autopaint_draws_one_frame_however_the_pointer_moves, pointer_wakes_that_change_no_paint_draw_no_frame, a_text_over_an_autopaint_blocks_it_across_its_layout_box, the_debug_geometry_check_agrees_with_an_autopaint_scene, the_project_option_sets_how_long_a_flash_lasts, a_picture_on_an_autopaint_shows_the_state_paint_as_its_tint_and_outline; local helpers: AUTO_PICTURES
- `tests/canvas/rt_canvas_autopaint/states.rs` (100) — 8 tests: the_hover_paint_shows_after_entry, the_enter_paint_shows_first_and_gives_way_with_the_pointer_still, the_exit_paint_shows_then_the_default, pressed_wins_over_hover, only_the_upper_of_two_overlapping_autopaints_changes, an_item_with_no_id_on_top_blocks_the_autopaint_beneath, two_uses_of_one_group_change_independently, a_hover_paint_with_a_wider_stroke_does_not_widen_the_hit_region
Surface: root declares 4 topic + 1 support module(s) as `#[path = "rt_canvas_autopaint/<name>.rs"] mod <name>;`; target name `rt_canvas_autopaint` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`). Support files: their items become `pub(super)` (counts above); root adds `use <support>::*;` per support file (1).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_autopaint.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | guard/script refs: none naming symbols (`grep -rn rt_canvas_autopaint tests/guards scripts .ai AGENTS.md src/docs .github Cargo.toml` → only the [[test]] stanza).

#### 4.9 `tests/canvas/rt_canvas_damage.rs` — 778 test LOC, 11 tests

- `tests/canvas/rt_canvas_damage.rs` (71) — root: header (`mod common`, `use`s); helpers: WIDTH, render, pixel, last, number; declares `unchanged_and_moved`, `transformed_boxes`, `grouped_boxes`, `resize`, `groups`
- `tests/canvas/rt_canvas_damage/unchanged_and_moved.rs` (107) — 3 tests: damage_changes_no_pixel, re_presenting_an_unchanged_scene_renders_nothing, a_moved_item_repaints_its_own_rectangle_and_not_the_window; local helpers: UNCHANGED, MOVED
- `tests/canvas/rt_canvas_damage/transformed_boxes.rs` (171) — 1 tests: a_transformed_items_box_is_its_mapped_extent_and_a_rim_of_pixels; local helpers: TRANSFORMED_BOXES
- `tests/canvas/rt_canvas_damage/grouped_boxes.rs` (199) — 1 tests: an_items_box_inside_a_scaling_group_is_its_mapped_extent_and_a_rim_of_pixels; local helpers: GROUPED_BOXES
- `tests/canvas/rt_canvas_damage/resize.rs` (102) — 3 tests: did_resize_is_false_until_the_surface_changes_and_then_true_once, a_first_publish_of_the_startup_size_is_not_a_resize, a_first_publish_of_another_size_is_one_resize; local helpers: RESIZE_POLL, scripted_resize
- `tests/canvas/rt_canvas_damage/groups.rs` (143) — 3 tests: replacing_a_group_damages_where_the_group_draws, a_moved_group_repaints_both_positions, turning_a_group_repaints_and_matches_a_full_redraw; local helpers: GROUP_REPLACED, GROUP_MOVED, GROUP_TURNED
Surface: root declares 5 topic + 0 support module(s) as `#[path = "rt_canvas_damage/<name>.rs"] mod <name>;`; target name `rt_canvas_damage` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: `src/docs/spec/app/06_canvas.md:1548` `turning_a_group_repaints_and_matches_a_full_redraw` → `tests/canvas/rt_canvas_damage/groups.rs` | include_str!/include_bytes! in moved code: none | prose refs (path only, stay valid): scripts/test-canvas-vulkan.sh:258,460; .ai/canvas-threading.md:1093.

#### 4.10 `tests/canvas/rt_canvas_graphics_thread.rs` — 619 test LOC, 20 tests

- `tests/canvas/rt_canvas_graphics_thread.rs` (77) — root: header (`mod common`, `use`s); helpers: run, run_with, run_env, run_env_timed, program, ONE_BOX, stat_field; declares `metal`, `image_lifetime`, `autopaint_windows`, `frame_pacing`
- `tests/canvas/rt_canvas_graphics_thread/metal.rs` (147) — 5 tests: the_metal_path_gives_one_completed_frame_per_changed_present, an_identical_re_present_draws_no_second_metal_frame, the_renderer_seam_reports_its_device_and_leaves_it_unselected, a_group_replaced_between_two_identical_presents_draws_a_second_frame, two_identical_presents_of_an_unchanged_group_draw_one_frame
- `tests/canvas/rt_canvas_graphics_thread/image_lifetime.rs` (106) — 3 tests: destroying_an_image_mid_frame_lets_the_frame_finish, the_frame_after_a_destroy_skips_the_picture, set_bytes_then_destroy_then_a_frame_draws_nothing; local helpers: FRAME_W, FRAME_H, picture_frame, rgba_at
- `tests/canvas/rt_canvas_graphics_thread/autopaint_windows.rs` (139) — 4 tests: auto_a_pointer_change_during_a_frame_belongs_to_the_next_frame, auto_a_present_during_an_enter_window_does_not_restart_it, auto_shutdown_completes_while_a_window_is_pending, auto_wait_frame_is_not_cut_short_by_a_window_ending; local helpers: AUTO_BUTTON, auto_states
- `tests/canvas/rt_canvas_graphics_thread/frame_pacing.rs` (162) — 8 tests: a_present_then_immediate_return_still_renders, canvas_mode_without_a_present_renders_nothing, a_static_scene_renders_once_and_does_not_spin, a_static_scene_under_a_wait_frame_loop_renders_once, display_hz_override_reaches_the_stats_line, an_identical_re_present_draws_no_second_frame, content_equal_scenes_with_nested_lists_draw_one_frame, sync_mode_gives_one_frame_per_changed_present
Surface: root declares 4 topic + 0 support module(s) as `#[path = "rt_canvas_graphics_thread/<name>.rs"] mod <name>;`; target name `rt_canvas_graphics_thread` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_graphics_thread.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include paths (one dir deeper: prefix `../`): `tests/canvas/rt_canvas_graphics_thread/frame_pacing.rs`: content_equal_scenes_with_nested_lists_draw_one_frame `"scenes/frame_skip_nested_lists.mfb"`→`"../scenes/frame_skip_nested_lists.mfb"` | prose refs (path only): .ai/canvas-threading.md:466,493,976.

#### 4.11 `tests/canvas/rt_canvas_group_ownership.rs` — 589 test LOC, 15 tests

- `tests/canvas/rt_canvas_group_ownership.rs` (138) — root: header (`mod common`, `use`s); helpers: stats_for, run_for, field, glyph_count, CHURN_200; tests kept (2): two_hundred_owning_cycles_return_group_bytes_to_baseline, the_font_fixture_is_a_real_truetype; declares `mid_frame`, `appended`, `binding`
- `tests/canvas/rt_canvas_group_ownership/mid_frame.rs` (153) — 2 tests: removing_a_group_mid_frame_keeps_the_font_until_the_frame_completes, removing_a_group_mid_frame_keeps_the_image_until_the_frame_completes; local helpers: REMOVE_MID_FRAME, REMOVE_FONT_MID_FRAME
- `tests/canvas/rt_canvas_group_ownership/appended.rs` (102) — 4 tests: a_loop_built_list_keeps_its_font_past_the_scope, an_appended_let_item_keeps_its_font_past_the_scope, two_appended_resources_are_both_kept, the_other_placing_collections_members_keep_their_font; local helpers: text_item, PRESENT_HELD, append_program, open_face, loop_built_body, let_item_body, two_resources_body, sibling_body, glyphs_for, assert_owned_like_control
- `tests/canvas/rt_canvas_group_ownership/binding.rs` (205) — 7 tests: a_group_does_not_close_an_image_the_scene_still_names, a_group_rebuilt_each_frame_keeps_its_font, the_rebuild_still_frees_the_buffers_it_retires, a_replacement_that_drops_the_image_closes_it, the_control_draws_with_the_binding_alive, the_group_keeps_the_font_past_its_scope, ownership_does_not_change_what_is_drawn; local helpers: DROPS_ITS_BINDING, KEEPS_ITS_BINDING, REBUILDS_EACH_FRAME, REPLACED_THEN_DROPPED, GROUP_AND_SCENE_SHARE
Surface: root declares 3 topic + 0 support module(s) as `#[path = "rt_canvas_group_ownership/<name>.rs"] mod <name>;`; target name `rt_canvas_group_ownership` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_group_ownership.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | the map measured the file as on disk, including bug-1092's 136 uncommitted added lines (`git diff --stat -- tests/canvas/rt_canvas_group_ownership.rs` → `136 ++++`), so `appended.rs` above holds bug-1092's four regression tests; Prerequisites requires those edits committed before Phase 2. Prose ref .ai/canvas-threading.md:478 (path only).

#### 4.12 `tests/canvas/rt_canvas_image_decode.rs` — 722 test LOC, 17 tests

- `tests/canvas/rt_canvas_image_decode.rs` (134) — root: header (`mod common`, `use`s); helpers: DECODER, decode, parse_decoder_line, assert_decodes, flat, DYNAMIC_HUFFMAN_PNG, unhex, dynamic_fixture_truth, ihdr, SIGNATURE; declares `pixel_formats`, `inflate_and_refusals`, `size_caps`, `png_builder`
- `tests/canvas/rt_canvas_image_decode/png_builder.rs` (167) — shared helpers (`pub(super)` on 11 items): W, H, truth, crc32, adler32, chunk, zlib_stored, png, pack_row, unfiltered, filtered_rgba
- `tests/canvas/rt_canvas_image_decode/pixel_formats.rs` (112) — 4 tests: every_png_filter_reconstructs_the_same_image, greyscale_at_every_bit_depth_scales_to_full_range, a_palette_index_names_an_entry_and_trns_gives_it_alpha, truecolour_and_greyscale_alpha_fill_the_channels_they_lack
- `tests/canvas/rt_canvas_image_decode/inflate_and_refusals.rs` (123) — 7 tests: an_interlaced_image_scatters_to_the_same_pixels_as_a_progressive_one, fixed_and_dynamic_huffman_blocks_both_inflate, a_file_that_is_not_a_png_is_refused_by_name, a_truncated_png_fails_rather_than_decoding_what_survived, a_header_no_png_could_have_is_refused_before_any_decoding, a_png_whose_zlib_adler32_is_wrong_is_refused, a_png_whose_huffman_tree_is_oversubscribed_is_refused; local helpers: ADAM7_X_ORIGIN, ADAM7_Y_ORIGIN, ADAM7_X_STEP, ADAM7_Y_STEP, adam7_rgba, FIXED_HUFFMAN_PNG, png_with_bad_adler32, OVERSUBSCRIBED_IDAT
- `tests/canvas/rt_canvas_image_decode/size_caps.rs` (199) — 6 tests: an_ihdr_declaring_more_pixels_than_any_image_may_have_is_refused_before_allocating, a_header_the_file_is_too_small_to_fill_is_refused_without_reading_the_pixels, an_idat_that_inflates_past_what_the_image_needs_is_refused, idat_accumulation_is_linear_in_the_chunk_count, a_stream_split_into_single_byte_idat_chunks_decodes_to_the_same_pixels, a_large_ordinary_image_is_well_inside_the_caps; local helpers: decode_bounded, header_only_png, zlib_zero_bomb, one_pixel_bomb_png, many_idat_png, rechunk_idat
Surface: root declares 3 topic + 1 support module(s) as `#[path = "rt_canvas_image_decode/<name>.rs"] mod <name>;`; target name `rt_canvas_image_decode` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`). Support files: their items become `pub(super)` (counts above); root adds `use <support>::*;` per support file (1).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_image_decode.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | refs: none beyond the [[test]] stanza.

#### 4.13 `tests/canvas/rt_canvas_golden.rs` — 1182 test LOC, 28 tests

- `tests/canvas/rt_canvas_golden.rs` (145) — root: header (`mod common`, `use`s); helpers: WIDTH, HEIGHT, golden_path, render_with_font, render, render_gpu_with_font, render_gpu, render_inner, GROUPS, POLYLINES; declares `comparators`, `blend_and_transforms`, `endcaps_and_corners`, `particles_and_pictures`, `ellipses`, `gradients`, `groups`, `polylines`
- `tests/canvas/rt_canvas_golden/comparators.rs` (104) — 5 tests: smiley_matches_its_reference_exactly, identical_frames_pass_both_comparators, a_one_step_perturbation_fails_exact_but_passes_tolerance, a_large_channel_delta_fails_tolerance, too_many_small_differences_fail_tolerance; local helpers: SMILEY
- `tests/canvas/rt_canvas_golden/blend_and_transforms.rs` (169) — 3 tests: blend_modes_match_their_reference_exactly, transforms_match_their_reference_exactly, the_gpu_draws_the_transform_scene_the_reference_shows; local helpers: BLEND_MODES, TRANSFORMS
- `tests/canvas/rt_canvas_golden/endcaps_and_corners.rs` (137) — 3 tests: endcaps_match_their_reference_exactly, the_gpu_draws_the_endcap_scene_the_reference_shows, corners_match_their_reference_exactly; local helpers: ENDCAPS, CORNERS
- `tests/canvas/rt_canvas_golden/particles_and_pictures.rs` (86) — 3 tests: particles_match_their_reference_exactly, the_gpu_draws_the_particle_scene_the_reference_shows, picture_shapes_match_their_reference_exactly; local helpers: PARTICLES, PICTURE_SHAPES
- `tests/canvas/rt_canvas_golden/ellipses.rs` (101) — 2 tests: ellipses_match_their_reference_exactly, the_gpu_draws_the_ellipse_scene_the_reference_shows; local helpers: ELLIPSES
- `tests/canvas/rt_canvas_golden/gradients.rs` (193) — 4 tests: gradients_match_their_reference_exactly, the_gpu_draws_the_gradient_scene_the_reference_shows, a_frame_past_the_gradient_stop_cap_declines_to_software, a_gradient_on_a_stroked_text_is_ignored; local helpers: GRADIENTS, GRADIENT_OVERFLOW
- `tests/canvas/rt_canvas_golden/groups.rs` (166) — 5 tests: a_scene_containing_a_group_reaches_the_gpu, group_affine_matches_its_reference_exactly, the_gpu_draws_the_group_affine_scene_the_reference_shows, groups_match_their_reference_exactly, the_gpu_draws_the_group_scene_the_reference_shows; local helpers: GROUP_SCENE, FLAT_SCENE, GROUP_AFFINE
- `tests/canvas/rt_canvas_golden/polylines.rs` (105) — 3 tests: polylines_match_their_reference_exactly, a_two_point_polyline_draws_the_line_between_its_points, the_gpu_draws_the_polyline_scene_the_reference_shows
Surface: root declares 8 topic + 0 support module(s) as `#[path = "rt_canvas_golden/<name>.rs"] mod <name>;`; target name `rt_canvas_golden` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: `src/docs/spec/app/06_canvas.md:1592` `the_gpu_draws_the_group_affine_scene_the_reference_shows` → `tests/canvas/rt_canvas_golden/groups.rs` | include paths (one dir deeper: prefix `../`): `tests/canvas/rt_canvas_golden/endcaps_and_corners.rs`: CORNERS `"scenes/rect_corners_on_black.mfb"`→`"../scenes/rect_corners_on_black.mfb"`; `tests/canvas/rt_canvas_golden/particles_and_pictures.rs`: PARTICLES `"scenes/particles_on_black.mfb"`→`"../scenes/particles_on_black.mfb"`, PICTURE_SHAPES `"scenes/picture_shapes_on_black.mfb"`→`"../scenes/picture_shapes_on_black.mfb"`; `tests/canvas/rt_canvas_golden/groups.rs`: GROUP_AFFINE `"scenes/group_affine_on_black.mfb"`→`"../scenes/group_affine_on_black.mfb"` | **scripts/test-canvas-vulkan.sh:541 and :682 `sed -n '/^const GROUPS: &str = r#"/,/^"#;$/p' tests/canvas/rt_canvas_golden.rs` (and POLYLINES)** — `GROUPS` and `POLYLINES` are pinned in the root, unindented, so the extraction keeps working. Prose refs: .ai/canvas-threading.md:1020, .ai/testing-gates.md:239,292 (path only).

#### 4.14 `tests/canvas/rt_canvas_metal.rs` — 1710 test LOC, 71 tests

- `tests/canvas/rt_canvas_metal.rs` (126) — root: header (`mod common`, `use`s); helpers: WIDTH, HEIGHT, Program, build, render, render_env, metal_built, lit, assert_metal_draws_like_software, assert_metal_draws_within; declares `primitives`, `fallback_and_transforms`, `paths_and_passes`, `groups_and_text`, `pictures_and_text_spacing`, `large_scenes`, `stars_fills_and_strokes`, `shapes_over_black`, `curves_and_buffer_passes`
- `tests/canvas/rt_canvas_metal/primitives.rs` (217) — 3 tests: rectangles_match_the_software_oracle_within_tolerance, the_full_primitive_set_matches_the_software_oracle, a_debug_metal_frame_reports_its_own_phases; local helpers: RECTANGLES, PRIMITIVES, stat
- `tests/canvas/rt_canvas_metal/fallback_and_transforms.rs` (191) — 3 tests: an_unsupported_scene_falls_back_to_the_software_renderer, a_group_with_a_matrix_draws_on_metal_and_matches_the_software_oracle, an_unevenly_transformed_item_in_a_shrinking_group_keeps_every_covered_pixel; local helpers: TOO_MANY_EDGES, TURNED_GROUP, HALO_IN_A_SHRINKING_GROUP
- `tests/canvas/rt_canvas_metal/paths_and_passes.rs` (198) — 4 tests: polylines_match_the_software_oracle_on_metal, shapes_turned_about_their_position_match_the_software_oracle, path_placement_matches_the_software_oracle, a_frame_whose_polygons_together_overflow_the_edge_region_draws_in_passes; local helpers: PAST_THE_FRAME_EDGE_REGION, POLYLINES, PIVOTED, PATH_PLACEMENT
- `tests/canvas/rt_canvas_metal/groups_and_text.rs` (214) — 4 tests: every_group_case_matches_the_software_oracle, a_diamond_renders_identically_to_the_flat_scene_it_stands_for, a_group_naming_an_absent_group_draws_nothing_on_the_gpu, every_glyph_of_a_text_run_draws_its_own_bitmap; local helpers: GROUPS, DIAMOND_GROUPED, DIAMOND_FLAT, ABSENT_GROUP, EMPTY_SCENE, TEXT_LINE
- `tests/canvas/rt_canvas_metal/pictures_and_text_spacing.rs` (121) — 2 tests: every_picture_path_matches_the_software_oracle, a_space_in_a_text_run_does_not_shift_the_draws_after_it; local helpers: PICTURES, SPACE_THEN_SHAPES
- `tests/canvas/rt_canvas_metal/large_scenes.rs` (105) — 12 tests: a_particle_system_draws_on_metal_and_matches_the_software_oracle, a_particle_system_in_a_turned_group_draws_on_metal, a_scene_past_four_thousand_quads_draws_on_metal, forty_thousand_polygon_edges_draw_on_metal, a_scene_past_four_thousand_gradient_stops_draws_on_metal, tile_atlas_regions_match_the_software_oracle_on_metal, a_tilemap_of_two_thousand_tiles_draws_on_metal, a_full_hd_background_draws_on_metal, a_layered_scene_draws_its_layers_on_metal, polygons_past_two_hundred_fifty_six_edges_draw_on_metal, a_sixty_four_thousand_edge_polygon_draws_on_metal, forty_thousand_opaque_polygon_edges_draw_on_metal; local helpers: TOO_MANY_FRAME_EDGES, PARTICLES, PARTICLES_GROUPED, FIVE_THOUSAND_QUADS, MANY_GRADIENTS, TILEMAP, TILE, BACKGROUND, LARGE_POLYGONS, LAYERED, HUGE_POLYGON, OPAQUE_FRAME_EDGES
- `tests/canvas/rt_canvas_metal/stars_fills_and_strokes.rs` (195) — 18 tests: a_long_chord_star_over_black_matches_the_software_oracle, a_three_hundred_one_point_long_chord_star_over_black_matches_the_software_oracle, a_short_edge_star_over_black_matches_the_software_oracle, a_short_edge_star_filled_non_zero_matches_the_software_oracle, both_fill_rules_match_the_software_oracle_on_metal, square_caps_match_the_software_oracle_on_metal, joins_match_the_software_oracle_on_metal, gradients_match_the_software_oracle_on_metal, path_clips_match_the_software_oracle_on_metal, corners_match_the_software_oracle_on_metal, transformed_path_clips_match_the_software_oracle_on_metal, picture_shapes_match_the_software_oracle_on_metal, texel_boundaries_on_pixel_centres_match_the_software_oracle_on_metal, a_hovered_autopaint_picture_matches_the_software_oracle_on_metal, dashes_match_the_software_oracle_on_metal, polygons_past_two_hundred_fifty_six_edges_over_black_match_the_software_oracle, small_polygons_over_black_match_the_software_oracle, rectangles_over_black_match_the_software_oracle; local helpers: STAR_251_97, STAR_301_97, STAR_301_2, STAR_301_2_NONZERO, FILL_RULES, SQUARE_CAPS, JOINS, GRADIENTS, PATH_CLIPS, CORNERS, TRANSFORMED_CLIPS, PICTURE_SHAPES, PICTURE_TEXEL_EDGE, DASHES, LARGE_POLYGONS_ON_BLACK, SMALL_POLYGONS_ON_BLACK, RECTANGLES_ON_BLACK
- `tests/canvas/rt_canvas_metal/shapes_over_black.rs` (167) — 13 tests: circles_over_black_match_the_software_oracle, rotated_ellipses_over_black_match_the_software_oracle, round_capped_lines_over_black_match_the_software_oracle, butt_capped_arcs_over_black_match_the_software_oracle, polygon_strokes_off_the_fixed_point_grid_over_black_match_the_software_oracle, polylines_over_black_match_the_software_oracle, polygons_in_groups_at_off_grid_offsets_over_black_match_the_software_oracle, stroked_and_capped_shapes_over_black_match_the_software_oracle, rectangles_with_edges_at_a_coverage_step_over_black_match_the_software_oracle, clips_with_edges_at_a_coverage_step_over_black_match_the_software_oracle, transformed_and_clipped_shapes_over_black_match_the_software_oracle, a_blended_item_stroked_under_the_fixed_point_step_matches_the_software_oracle, a_hovered_autopaint_matches_the_software_oracle_on_metal; local helpers: CIRCLES_ON_BLACK, ELLIPSES_ON_BLACK, LINES_ON_BLACK, ARCS_ON_BLACK, STROKED_POLYGONS_ON_BLACK, POLYLINES_ON_BLACK, GROUPED_POLYGONS_ON_BLACK, STROKED_SHAPES_ON_BLACK, RECTANGLES_AT_A_STEP_ON_BLACK, CLIPS_AT_A_STEP_ON_BLACK, TRANSFORMED_SHAPES_ON_BLACK, BLENDED_HAIRLINE_STROKE, AUTOPAINT
- `tests/canvas/rt_canvas_metal/curves_and_buffer_passes.rs` (203) — 12 tests: the_curve_shapes_match_the_software_oracle_on_metal, a_hovered_autopaint_over_a_curve_matches_the_software_oracle_on_metal, every_curve_stroke_and_fill_matches_the_software_oracle_on_metal, turned_and_grouped_curves_match_the_software_oracle_on_metal, pictures_in_curved_shapes_match_the_software_oracle_on_metal, the_pieces_reach_the_gpu_buffer_with_their_residuals, a_polygon_vertex_on_a_pixel_row_is_counted_once_on_metal, curve_pieces_are_counted_two_slots_each_against_the_edge_region, a_frame_past_one_buffers_items_draws_in_passes_on_metal, a_frame_past_one_buffers_gradient_stops_draws_in_passes_on_metal, a_frame_past_one_buffers_texels_draws_in_passes_on_metal, a_group_drawn_again_in_a_later_pass_is_drawn_in_paint_order_on_metal; local helpers: CURVES, CURVE_STROKES, CURVES_PLACED, CURVE_PICTURES, CURVE_RESIDUALS, VERTEX_ON_A_ROW, PIECE_RINGS, MANY_ITEMS, MANY_GRADIENT_STOPS, MANY_TEXELS, PASSES_GROUPED
Surface: root declares 9 topic + 0 support module(s) as `#[path = "rt_canvas_metal/<name>.rs"] mod <name>;`; target name `rt_canvas_metal` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: `src/docs/spec/app/06_canvas.md:1593` `a_group_with_a_matrix_draws_on_metal_and_matches_the_software_oracle` → `tests/canvas/rt_canvas_metal/fallback_and_transforms.rs` | include paths (one dir deeper: prefix `../`): `tests/canvas/rt_canvas_metal/large_scenes.rs`: TOO_MANY_FRAME_EDGES `"scenes/forty_thousand_edges.mfb"`→`"../scenes/forty_thousand_edges.mfb"`, PARTICLES `"scenes/particles_on_black.mfb"`→`"../scenes/particles_on_black.mfb"`, PARTICLES_GROUPED `"scenes/particles_grouped_on_black.mfb"`→`"../scenes/particles_grouped_on_black.mfb"`, FIVE_THOUSAND_QUADS `"scenes/five_thousand_quads.mfb"`→`"../scenes/five_thousand_quads.mfb"`, MANY_GRADIENTS `"scenes/many_gradients.mfb"`→`"../scenes/many_gradients.mfb"`, TILEMAP `"scenes/tilemap.mfb"`→`"../scenes/tilemap.mfb"`, TILE `"scenes/tile.mfb"`→`"../scenes/tile.mfb"`, BACKGROUND `"scenes/background.mfb"`→`"../scenes/background.mfb"`, LARGE_POLYGONS `"scenes/large_polygons.mfb"`→`"../scenes/large_polygons.mfb"`, LAYERED `"scenes/layered.mfb"`→`"../scenes/layered.mfb"`, HUGE_POLYGON `"scenes/huge_polygon.mfb"`→`"../scenes/huge_polygon.mfb"`, OPAQUE_FRAME_EDGES `"scenes/forty_thousand_opaque_edges.mfb"`→`"../scenes/forty_thousand_opaque_edges.mfb"`; `tests/canvas/rt_canvas_metal/paths_and_passes.rs`: PAST_THE_FRAME_EDGE_REGION `"scenes/past_the_edge_region.mfb"`→`"../scenes/past_the_edge_region.mfb"`; `tests/canvas/rt_canvas_metal/stars_fills_and_strokes.rs`: STAR_251_97 `"scenes/star_251_97_on_black.mfb"`→`"../scenes/star_251_97_on_black.mfb"`, STAR_301_97 `"scenes/star_301_97_on_black.mfb"`→`"../scenes/star_301_97_on_black.mfb"`, STAR_301_2 `"scenes/star_301_2_on_black.mfb"`→`"../scenes/star_301_2_on_black.mfb"`, STAR_301_2_NONZERO `"scenes/star_301_2_nonzero_on_black.mfb"`→`"../scenes/star_301_2_nonzero_on_black.mfb"`, FILL_RULES `"scenes/fill_rules_on_black.mfb"`→`"../scenes/fill_rules_on_black.mfb"`, SQUARE_CAPS `"scenes/square_caps_on_black.mfb"`→`"../scenes/square_caps_on_black.mfb"`, JOINS `"scenes/joins_on_black.mfb"`→`"../scenes/joins_on_black.mfb"`, GRADIENTS `"scenes/gradients_on_black.mfb"`→`"../scenes/gradients_on_black.mfb"`, PATH_CLIPS `"scenes/path_clips_on_black.mfb"`→`"../scenes/path_clips_on_black.mfb"`, CORNERS `"scenes/rect_corners_on_black.mfb"`→`"../scenes/rect_corners_on_black.mfb"`, TRANSFORMED_CLIPS `"scenes/transformed_clips_on_black.mfb"`→`"../scenes/transformed_clips_on_black.mfb"`, PICTURE_SHAPES `"scenes/picture_shapes_on_black.mfb"`→`"../scenes/picture_shapes_on_black.mfb"`, PICTURE_TEXEL_EDGE `"scenes/picture_texel_edge.mfb"`→`"../scenes/picture_texel_edge.mfb"`, DASHES `"scenes/dashes_on_black.mfb"`→`"../scenes/dashes_on_black.mfb"`, LARGE_POLYGONS_ON_BLACK `"scenes/large_polygons_on_black.mfb"`→`"../scenes/large_polygons_on_black.mfb"`, SMALL_POLYGONS_ON_BLACK `"scenes/small_polygons_on_black.mfb"`→`"../scenes/small_polygons_on_black.mfb"`, RECTANGLES_ON_BLACK `"scenes/rectangles_on_black.mfb"`→`"../scenes/rectangles_on_black.mfb"`; `tests/canvas/rt_canvas_metal/shapes_over_black.rs`: CIRCLES_ON_BLACK `"scenes/circles_on_black.mfb"`→`"../scenes/circles_on_black.mfb"`, ELLIPSES_ON_BLACK `"scenes/ellipses_on_black.mfb"`→`"../scenes/ellipses_on_black.mfb"`, LINES_ON_BLACK `"scenes/lines_on_black.mfb"`→`"../scenes/lines_on_black.mfb"`, ARCS_ON_BLACK `"scenes/arcs_on_black.mfb"`→`"../scenes/arcs_on_black.mfb"`, STROKED_POLYGONS_ON_BLACK `"scenes/stroked_polygons_on_black.mfb"`→`"../scenes/stroked_polygons_on_black.mfb"`, POLYLINES_ON_BLACK `"scenes/polylines_on_black.mfb"`→`"../scenes/polylines_on_black.mfb"`, GROUPED_POLYGONS_ON_BLACK `"scenes/grouped_polygons_on_black.mfb"`→`"../scenes/grouped_polygons_on_black.mfb"`, STROKED_SHAPES_ON_BLACK `"scenes/stroked_shapes_on_black.mfb"`→`"../scenes/stroked_shapes_on_black.mfb"`, RECTANGLES_AT_A_STEP_ON_BLACK `"scenes/rectangles_at_a_step_on_black.mfb"`→`"../scenes/rectangles_at_a_step_on_black.mfb"`, CLIPS_AT_A_STEP_ON_BLACK `"scenes/clips_at_a_step_on_black.mfb"`→`"../scenes/clips_at_a_step_on_black.mfb"`, TRANSFORMED_SHAPES_ON_BLACK `"scenes/transformed_shapes_on_black.mfb"`→`"../scenes/transformed_shapes_on_black.mfb"`, BLENDED_HAIRLINE_STROKE `"scenes/blended_hairline_stroke.mfb"`→`"../scenes/blended_hairline_stroke.mfb"`, AUTOPAINT `"scenes/autopaint_on_black.mfb"`→`"../scenes/autopaint_on_black.mfb"`; `tests/canvas/rt_canvas_metal/curves_and_buffer_passes.rs`: CURVES `"scenes/curves.mfb"`→`"../scenes/curves.mfb"`, CURVE_STROKES `"scenes/curve_strokes_on_black.mfb"`→`"../scenes/curve_strokes_on_black.mfb"`, CURVES_PLACED `"scenes/curves_placed_on_black.mfb"`→`"../scenes/curves_placed_on_black.mfb"`, CURVE_PICTURES `"scenes/curve_pictures_on_black.mfb"`→`"../scenes/curve_pictures_on_black.mfb"`, CURVE_RESIDUALS `"scenes/curve_residuals_on_black.mfb"`→`"../scenes/curve_residuals_on_black.mfb"`, VERTEX_ON_A_ROW `"scenes/vertex_on_a_pixel_row.mfb"`→`"../scenes/vertex_on_a_pixel_row.mfb"`, MANY_ITEMS `"scenes/many_items.mfb"`→`"../scenes/many_items.mfb"`, MANY_GRADIENT_STOPS `"scenes/many_gradient_stops.mfb"`→`"../scenes/many_gradient_stops.mfb"`, MANY_TEXELS `"scenes/many_texels.mfb"`→`"../scenes/many_texels.mfb"`, PASSES_GROUPED `"scenes/passes_grouped.mfb"`→`"../scenes/passes_grouped.mfb"` | every scene const moves with its only test; `WIDTH/HEIGHT/Program/build/render*/metal_built/lit/assert_metal_draws_*` stay root. Prose refs: scripts/test-canvas-gpu-rows.sh:7, scripts/test-canvas-vulkan.sh:4, .ai/testing-gates.md:314, .ai/canvas-threading.md:913 (path/target only).

#### 4.15 `tests/canvas/rt_canvas_font.rs` — 1763 test LOC, 33 tests

- `tests/canvas/rt_canvas_font.rs` (154) — root: header (`mod common`, `use`s); helpers: run, run_files, ERR_BAD_FONT_FILE, RESOURCE_CLOSED, run_with_font, run_with_font_frame, MEASURE, collection; declares `loading`, `measure`, `transforms_and_cache`, `outlines`, `pen_walk`, `gpu`, `bounds`, `collection_faces`, `collections`, `fixture`, `render`
- `tests/canvas/rt_canvas_font/fixture.rs` (96; no `use super::*;`, see §3) — shared helpers (`pub(super)` on 3 items): minimal_truetype, truetype_fixture, truetype_fixture_with
- `tests/canvas/rt_canvas_font/render.rs` (137) — shared helpers (`pub(super)` on 9 items, 2 fields): WIDTH, render, render_with, render_env, render_env_files, stat, pixel, FontRun, run_fonts_bounded
- `tests/canvas/rt_canvas_font/loading.rs` (189) — 4 tests: load_font_accepts_truetype_outlines_and_refuses_every_other_container, a_missing_path_is_not_reported_as_a_bad_font, a_second_destroy_font_raises_resource_closed, a_units_per_em_outside_the_formats_range_is_refused_at_load; local helpers: PROGRAM, ERR_NOT_FOUND
- `tests/canvas/rt_canvas_font/measure.rs` (128) — 5 tests: measure_text_scales_the_fonts_own_metrics, descent_is_reported_positive_though_the_file_stores_it_negative, measure_text_sums_a_mixed_run_in_font_units_then_scales_once, measuring_a_closed_font_raises_resource_closed, a_font_past_the_renderers_table_still_measures
- `tests/canvas/rt_canvas_font/transforms_and_cache.rs` (215) — 4 tests: text_inside_a_translated_group_draws_at_the_offset, a_rotated_text_run_draws_rotated, a_transformed_text_run_reaches_the_gpu_and_matches_the_oracle, two_hundred_presents_through_one_font_leave_the_glyph_cache_where_they_found_it; local helpers: ROTATED_TEXT, UPRIGHT_TEXT
- `tests/canvas/rt_canvas_font/outlines.rs` (161) — 3 tests: transparency_fades_text_and_stroked_text, a_glyph_outline_renders_where_its_own_coordinates_put_it, a_composite_glyph_draws_every_placed_component
- `tests/canvas/rt_canvas_font/pen_walk.rs` (168) — 5 tests: a_string_advances_the_pen_between_glyphs, every_live_font_draws_however_many_are_loaded, a_filled_run_places_every_glyph_at_its_own_snapped_pen, a_stroked_run_places_every_glyph_at_its_own_pen, verify_mode_agrees_with_the_filled_run; local helpers: WALK_TEXT, WALK_SIZE, WALK_PENS, walk_scene, lit_column_runs
- `tests/canvas/rt_canvas_font/gpu.rs` (191) — 4 tests: text_whose_font_was_destroyed_draws_nothing, text_on_the_gpu_matches_the_software_oracle, a_shape_after_a_glyph_run_matches_the_software_oracle, eviction_frees_unpinned_glyphs_and_changes_no_pixel; local helpers: GPU_TEXT, GPU_TEXT_THEN_SHAPE, EVICTION
- `tests/canvas/rt_canvas_font/bounds.rs` (130) — 4 tests: a_cmap_group_count_is_bounded_by_the_table_that_holds_it, a_cmap_group_count_past_the_end_of_the_file_is_bounded_by_the_file, a_glyph_whose_bitmap_would_exceed_the_raster_cap_is_skipped, a_display_sized_glyph_is_well_inside_the_raster_cap; local helpers: draw_a, lit_pixels, cmap_subtable_at_eof_claiming_everything
- `tests/canvas/rt_canvas_font/collection_faces.rs` (145) — 2 tests: load_font_chooses_a_collection_face_by_name, a_system_collection_face_loads_by_post_script_name; local helpers: ERR_FACE_NOT_FOUND, with_table, name_table, utf16be, named_face
- `tests/canvas/rt_canvas_font/collections.rs` (83) — 2 tests: a_collection_loads_its_first_face, a_collection_that_runs_past_its_end_is_refused; local helpers: DRAW_A
Surface: root declares 9 topic + 2 support module(s) as `#[path = "rt_canvas_font/<name>.rs"] mod <name>;`; target name `rt_canvas_font` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`). Support files: their items become `pub(super)` (counts above); root adds `use <support>::*;` per support file (2).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_font.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | prose refs naming the fixture BUILDER, which moves to `fixture.rs`, are repointed to `tests/canvas/rt_canvas_font/fixture.rs`: `scripts/remote-common.sh:47`, `scripts/test-canvas-vulkan.sh:88`, and `scripts/man-run-examples.sh:260` (which says `tests/rt_canvas_font.rs`, a path that does not exist today: `ls tests/rt_canvas_font.rs` → No such file — a pre-existing stale reference fixed here); live planning/bug docs: `planning/completed/plan-162-B-kerning.md:59` (`tests/canvas/rt_canvas_font.rs:truetype_fixture`) and `:164` ("extend the fixture builder" in `tests/canvas/rt_canvas_font.rs`) → `tests/canvas/rt_canvas_font/fixture.rs`; `bugs/bug-1091-every-text-geometry-build-copies-the-whole-font-file.md:130` ("Add a test to `tests/canvas/rt_canvas_font.rs`") → `tests/canvas/rt_canvas_font/transforms_and_cache.rs`, the topic holding the sibling glyph-cache test `two_hundred_presents_through_one_font_leave_the_glyph_cache_where_they_found_it`. `bugs/bug-1091-…:41` cites `tests/canvas/rt_canvas_hit.rs:FIXTURE_TTF`, which stays in the `rt_canvas_hit` root — valid. Archived docs under `bugs/completed/` and `planning/completed/` (e.g. `bugs/completed/bug-757-composite-glyphs-draw-nothing.md`) are history and are not edited.

#### 4.16 `tests/canvas/rt_canvas_curves.rs` — 2432 test LOC, 40 tests

- `tests/canvas/rt_canvas_curves.rs` (165) — root: header (`mod common`, `use`s); helpers: PRELUDE, stroke_paint, RED, image; declares `reduction`, `chains_and_hashing`, `nurbs`, `flatten`, `pieces`, `curve_polygon`, `gpu_and_joins`, `caps_and_dashes`, `placement_and_paint`, `picture_clips`, `picture_extent`, `driver`, `curve_model`, `scene`, `nurbs_model`
- `tests/canvas/rt_canvas_curves/driver.rs` (200) — shared helpers (`pub(super)` on 6 items): EPS, chunk, DRIVER, driver, run, value
- `tests/canvas/rt_canvas_curves/curve_model.rs` (193) — shared helpers (`pub(super)` on 18 items, 2 methods, 2 fields): Piece, pieces, marks, Curve, poly, rational, spell, reduce, chain, piece_at, distance, piece_distance, segment_distance, dense_deviation, assert_monotone, assert_connected, SQRT_HALF, cubic_s
- `tests/canvas/rt_canvas_curves/scene.rs` (141) — shared helpers (`pub(super)` on 9 items, 6 methods, 3 fields): WIDTH, HEIGHT, Scene, press, scene, scene_on, BLACK, WHITE, BLUE
- `tests/canvas/rt_canvas_curves/nurbs_model.rs` (97) — shared helpers (`pub(super)` on 4 items, 5 methods, 4 fields): float_source, float_list, point_list, Nurbs
- `tests/canvas/rt_canvas_curves/reduction.rs` (140) — 5 tests: canvas_curve_a_quadratic_reduces_to_itself_cut_at_its_turn, canvas_curve_a_rational_quadratic_circle_arc_stays_exact, canvas_curve_a_cubic_is_fitted_within_the_tolerance, canvas_curve_the_tolerance_follows_the_shapes_own_transform, canvas_curve_high_degree_and_rational_cubics_reduce
- `tests/canvas/rt_canvas_curves/chains_and_hashing.rs` (196) — 6 tests: canvas_curve_adversarial_curves_stay_within_the_tolerance, canvas_curve_validity, canvas_curve_a_chain_joins_its_curves, canvas_curve_a_contour_is_closed_and_a_chain_is_not, canvas_curve_a_chain_is_moved_and_its_marks_are_based, canvas_curve_shapes_hash_every_point_and_weight
- `tests/canvas/rt_canvas_curves/nurbs.rs` (176) — 3 tests: canvas_nurbs_a_circle_converts_to_four_exact_arcs, canvas_nurbs_a_b_spline_converts_to_the_beziers_of_its_spans, canvas_nurbs_invalid_input_returns_an_empty_list
- `tests/canvas/rt_canvas_curves/flatten.rs` (97) — 2 tests: canvas_flatten_stays_within_its_tolerance, canvas_flatten_points_build_a_polygon; local helpers: polyline_distance
- `tests/canvas/rt_canvas_curves/pieces.rs` (167) — 2 tests: canvas_piece_nearest_matches_a_brute_force_search, canvas_piece_crossing_is_on_the_piece
- `tests/canvas/rt_canvas_curves/curve_polygon.rs` (204) — 5 tests: canvas_curve_polygon_circle_matches_a_circle, canvas_curve_polygon_moves_with_its_position, canvas_curve_polygon_fills_by_its_rule, canvas_curve_polygon_holes_follow_the_rule, canvas_curve_polygon_closes_and_a_poly_curve_has_no_interior
- `tests/canvas/rt_canvas_curves/gpu_and_joins.rs` (160) — 4 tests: canvas_curve_frames_are_drawn_on_the_gpu, canvas_curve_stroke_has_no_join_where_the_curve_is_smooth, canvas_poly_curve_corner_takes_the_join, canvas_poly_curve_strokes_its_implied_segment
- `tests/canvas/rt_canvas_curves/caps_and_dashes.rs` (127) — 3 tests: canvas_poly_curve_that_returns_has_two_caps_not_a_join, canvas_poly_curve_dashes_follow_arc_length, canvas_curve_polygon_dashes_restart_at_each_contour; local helpers: pixel_of
- `tests/canvas/rt_canvas_curves/placement_and_paint.rs` (187) — 5 tests: canvas_curve_shapes_turn_about_their_position, canvas_curve_shapes_in_groups, canvas_curve_shapes_are_clipped_in_their_own_space, canvas_curve_shapes_take_the_whole_paint, canvas_curve_shapes_are_hit_where_they_are_drawn; local helpers: SHAPES
- `tests/canvas/rt_canvas_curves/picture_clips.rs` (124) — 2 tests: canvas_picture_curve_clip_is_within_the_tolerance_of_the_curve, canvas_picture_curve_clip_modes_and_the_paints_clip; local helpers: coverage_of, edge_gap
- `tests/canvas/rt_canvas_curves/picture_extent.rs` (107) — 3 tests: canvas_picture_curve_holes_and_outline, canvas_picture_curve_fits_the_true_extent, canvas_picture_on_a_poly_curve_draws_nothing
Surface: root declares 11 topic + 4 support module(s) as `#[path = "rt_canvas_curves/<name>.rs"] mod <name>;`; target name `rt_canvas_curves` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`). Support files: their items become `pub(super)` (counts above); root adds `use <support>::*;` per support file (4).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_curves.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | `helper`/`driver` read `src/…` via `env!("CARGO_MANIFEST_DIR")` at run time (absolute; unaffected). `Scene` has two `impl Scene` blocks (L861-902, L1133-1175): both go to `scene.rs`. `Curve`/`Nurbs`/`Scene` methods called from topics need `pub(super)` (counts above). Refs: none beyond the [[test]] stanza.

#### 4.17 `tests/canvas/rt_canvas_particles.rs` — 1214 test LOC, 29 tests

- `tests/canvas/rt_canvas_particles.rs` (81) — root: header (`mod common`, `use`s); helpers: Run, run, run_inner, report, answer, number; declares `system_lifecycle`, `model`, `templates`, `template_kinds_and_gpu`, `damage_and_cache`, `render`
- `tests/canvas/rt_canvas_particles/render.rs` (134) — shared helpers (`pub(super)` on 8 items): WIDTH, HEIGHT, draw, pixel, lit, stat, SCENE_PRELUDE, scene
- `tests/canvas/rt_canvas_particles/system_lifecycle.rs` (211) — 5 tests: a_scene_holding_a_system_is_presented_copied_and_dropped, presenting_a_system_leaks_nothing, every_record_is_built_and_every_shape_is_an_emitter, a_lifetime_of_zero_raises_from_present, sixty_frames_follow_the_programs_clock; local helpers: run_with_font, TEMPLATES, TYPE_SET
- `tests/canvas/rt_canvas_particles/model.rs` (146) — 8 tests: the_same_seed_and_slot_give_the_same_draw, a_slot_is_dead_before_birth_and_after_its_lifetime, a_burst_is_alive_after_each_period_and_dead_between, a_vanishing_drag_is_no_drag, a_disc_is_sampled_evenly_over_its_area, an_emitter_is_placed_and_turned_by_its_own_fields, a_stroke_emitter_samples_its_curve, a_polygon_emitter_samples_only_its_inside; local helpers: helper_source, MODEL_DRIVER, model, model_case
- `tests/canvas/rt_canvas_particles/templates.rs` (151) — 6 tests: a_particle_is_its_template_in_a_group_node, a_moved_particle_is_the_template_moved, a_turned_particle_is_its_template_in_a_turned_group_node, a_particle_scaled_to_nothing_is_not_drawn, a_system_in_a_turned_group_is_turned_with_it, the_tint_multiplies_and_fades_in_linear_light; local helpers: TILTED
- `tests/canvas/rt_canvas_particles/template_kinds_and_gpu.rs` (117) — 5 tests: a_text_template_is_placed_scaled_and_tinted, an_autopaint_template_draws_its_own_paint, curve_shapes_are_emitters, a_thousand_particles_are_one_scene_index, a_system_in_a_group_reaches_the_gpu; local helpers: THOUSAND
- `tests/canvas/rt_canvas_particles/damage_and_cache.rs` (144) — 5 tests: damage_of_a_system_is_bounded_by_its_particles, damage_of_an_item_beside_a_system_is_the_items, a_plain_particles_record_is_not_cached, ordinary_items_draw_the_same_beside_a_system, a_text_particles_record_is_still_cached_and_its_glyphs_drawn; local helpers: STREAM_OVER_BACKGROUND, damage, MIXED
- `tests/canvas/rt_canvas_particles/model_driver.mfb` (216 MFBASIC LOC) — the `MODEL_DRIVER` raw string's bytes (see Sweeps); `model.rs` LOC above counts the const as 1 line.
Surface: root declares 5 topic + 1 support module(s) as `#[path = "rt_canvas_particles/<name>.rs"] mod <name>;`; target name `rt_canvas_particles` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`). Support files: their items become `pub(super)` (counts above); root adds `use <support>::*;` per support file (1).
Sweeps: citations: none (`grep -rnoE "tests/canvas/rt_canvas_particles.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`) | include_str!/include_bytes! in moved code: none | **MODEL_DRIVER is one 250-LOC raw-string const (L256-505)**: no file holding it can be ≤250 by pure motion, so it takes the decided natural extraction (§Decisions): `const MODEL_DRIVER: &str = include_str!("model_driver.mfb");` in `model.rs`, with the raw string's bytes (between `r#"` and `"#`, i.e. L256's `\n` through L504's `END FUNC\n`) moved verbatim to `tests/canvas/rt_canvas_particles/model_driver.mfb` (216 MFBASIC LOC by `span.py`; a test file because it is under `tests/`). The path is relative to `model.rs`, which sits beside the `.mfb`. `MODEL_DRIVER`'s only reader is the `{MODEL_DRIVER}` capture in `model()`'s `format!` (`grep -n MODEL_DRIVER tests/canvas/rt_canvas_particles.rs` → L256 decl, L512 use), and an `include_str!` const is an ordinary `&'static str` there. Byte identity of the string is checked in Phase 4. No harness compiles stray `.mfb` files under `tests/canvas/` (`scripts/test-canvas-gpu-rows.sh` and `scripts/test-canvas-vulkan.sh` read only `tests/canvas/scenes/`: `grep -n 'tests/canvas/scenes\|SCENES=' scripts/test-canvas-gpu-rows.sh scripts/test-canvas-vulkan.sh`). `helper_source` reads src/ via CARGO_MANIFEST_DIR at run time (unaffected).

#### 4.18 `tests/canvas/rt_canvas_rasteriser.rs` — 4575 test LOC, 109 tests

- `tests/canvas/rt_canvas_rasteriser.rs` (188) — root: header (`mod common`, `use`s); helpers: WIDTH, HEIGHT, render, render_stdout, pixel, scene, RING_HELPERS, stat, ramp, lit, NO_T, focal, ramp_box; declares `basic_shapes`, `cache_and_polygons`, `fill_rules_and_square_caps`, `joins`, `ellipse_dashes`, `dash_patterns`, `clips`, `path_clips`, `blend_and_transparency`, `distance_correction_measurement`, `transforms`, `caps`, `ellipse_newton_measurement`, `ellipses`, `corner_rule_radii`, `corner_rule_binding`, `placement`, `path_placement`, `group_affine`, `gradients`, `gradient_strokes_and_order`, `gradient_transforms`, `gradient_focal`, `group_presents`, `group_rendering`, `group_retirement`, `resource_lifetime_mid_frame`, `ellipse_newton_solvers`
- `tests/canvas/rt_canvas_rasteriser/basic_shapes.rs` (196) — 7 tests: rectangle_fills_its_exact_span, an_arc_swept_to_pi_reaches_its_end_cap, circle_is_round_and_antialiased, arc_sweeps_clockwise_from_positive_x, translucent_fill_blends_in_linear_space, polygon_fills_its_interior, rounded_rect_corners_are_cut
- `tests/canvas/rt_canvas_rasteriser/cache_and_polygons.rs` (174) — 5 tests: rendering_is_byte_reproducible, cache_hit_skips_geometry_generation, polygons_sharing_a_header_keep_their_own_points, a_polygon_draws_each_of_its_rings, a_polygon_hash_covers_its_rings_and_rule
- `tests/canvas/rt_canvas_rasteriser/fill_rules_and_square_caps.rs` (194) — 3 tests: non_zero_fills_by_winding_and_even_odd_by_parity, a_convex_ring_draws_the_same_under_both_rules, a_square_cap_extends_the_stroke_square_past_each_end; local helpers: PENTAGRAM
- `tests/canvas/rt_canvas_rasteriser/joins.rs` (187) — 2 tests: a_join_shapes_every_corner_of_a_stroke, a_dash_crossing_a_corner_stays_one_dash
- `tests/canvas/rt_canvas_rasteriser/ellipse_dashes.rs` (100) — 2 tests: a_dashed_ellipse_is_cut_by_arc_length, an_ellipse_dash_table_is_within_a_tenth_of_a_pixel; local helpers: ellipse_arc_length, ellipse_angle_at
- `tests/canvas/rt_canvas_rasteriser/dash_patterns.rs` (170) — 4 tests: a_dashed_line_draws_its_pattern, a_dash_end_takes_the_cap, each_ring_restarts_the_pattern_and_a_circle_starts_at_plus_x, a_rounded_rect_and_an_arc_start_their_dashes_where_the_contract_says
- `tests/canvas/rt_canvas_rasteriser/clips.rs` (140) — 5 tests: a_clip_cuts_a_circle_in_half, a_fractional_clip_edge_is_antialiased, an_empty_clip_is_no_clip_and_a_clip_enclosing_nothing_hides_the_item, a_clip_outside_the_item_draws_nothing, a_clip_larger_than_the_item_changes_nothing
- `tests/canvas/rt_canvas_rasteriser/path_clips.rs` (135) — 3 tests: a_path_clip_draws_only_inside_its_rings, a_sheared_clip_edge_matches_a_sheared_shape_edge, a_rectangle_clip_draws_the_same_through_either_path; local helpers: clip_ring
- `tests/canvas/rt_canvas_rasteriser/blend_and_transparency.rs` (125) — 3 tests: transparency_fades_fill_stroke_and_gradient_alphas, each_blend_mode_composites_to_its_own_values, blend_mode_normal_is_identical_to_an_unset_blend
- `tests/canvas/rt_canvas_rasteriser/distance_correction_measurement.rs` (85) — 1 tests: measure_the_transformed_distance_correction
- `tests/canvas/rt_canvas_rasteriser/transforms.rs` (151) — 5 tests: a_rotated_rectangle_lands_where_the_matrix_says, a_uniform_scale_scales_the_shape_and_its_stroke, an_all_zero_transform_is_identical_to_no_transform, a_singular_transform_renders_untransformed, a_rotated_shape_is_not_clipped_to_its_untransformed_bounds
- `tests/canvas/rt_canvas_rasteriser/caps.rs` (141) — 5 tests: a_butt_cap_stops_at_the_endpoint_and_a_round_cap_does_not, a_zero_length_line_is_a_dot_only_when_round_capped, a_round_capped_arc_caps_its_sweep_ends_and_a_butt_one_does_not, a_round_arc_cap_at_the_bounds_extreme_is_not_clipped, a_full_circle_arc_is_identical_with_either_cap
- `tests/canvas/rt_canvas_rasteriser/ellipse_newton_measurement.rs` (157) — 1 tests: measure_the_ellipse_newton_iteration_count
- `tests/canvas/rt_canvas_rasteriser/ellipses.rs` (204) — 6 tests: an_ellipse_with_equal_radii_is_identical_to_a_circle, an_axis_aligned_ellipse_covers_its_extent, rotating_an_ellipse_by_a_quarter_turn_swaps_its_axes, an_ellipse_is_turned_by_its_transform, an_ellipse_with_a_zero_radius_draws_nothing, a_stroked_ellipse_is_hollow
- `tests/canvas/rt_canvas_rasteriser/corner_rule_radii.rs` (108) — 3 tests: corner_rule_every_meaning_of_a_radius_draws_the_corner_it_names, corner_rule_a_one_pixel_rectangle_is_still_one_pixel, corner_rule_only_the_named_corner_is_rounded; local helpers: corner_reach, top_left_rounded
- `tests/canvas/rt_canvas_rasteriser/corner_rule_binding.rs` (153) — 4 tests: corner_rule_bind_names_the_point_position_is, corner_rule_a_centre_bound_rectangle_spins_in_place, corner_rule_the_flattened_outline_is_within_its_bound_of_the_exact_one, corner_rule_the_surface_size_is_whole_pixels_held_as_floats
- `tests/canvas/rt_canvas_rasteriser/placement.rs` (152) — 4 tests: placement_a_transform_turns_a_shape_about_its_position, placement_a_transforms_translation_adds_to_the_position, placement_a_clipped_gradient_shape_moves_whole_with_its_position, placement_a_transform_with_no_area_leaves_the_shape_at_its_position
- `tests/canvas/rt_canvas_rasteriser/path_placement.rs` (201) — 3 tests: path_placement_points_turn_about_the_position, path_placement_a_clipped_polygon_moves_whole_with_its_position, path_placement_a_coloured_polyline_at_a_position_equals_its_points_moved; local helpers: NO_TRANSFORM
- `tests/canvas/rt_canvas_rasteriser/group_affine.rs` (138) — 5 tests: group_affine_a_group_turns_its_items_about_its_position, group_affine_a_nested_group_composes_with_its_parent, group_affine_a_scaled_group_scales_its_strokes, group_affine_a_transform_with_no_area_leaves_the_group_as_built, group_affine_a_transforms_translation_adds_to_the_position; local helpers: WHITE_BOX, QUARTER
- `tests/canvas/rt_canvas_rasteriser/gradients.rs` (224) — 6 tests: gradients_sharing_a_header_keep_their_own_stops, a_gradient_with_fewer_than_two_stops_is_byte_identical_to_a_flat_fill, a_linear_gradient_interpolates_between_its_stops, a_radial_gradient_runs_outward_from_its_centre, gradients_draw_their_own_ramps, a_zero_length_gradient_axis_takes_the_first_stops_colour
- `tests/canvas/rt_canvas_rasteriser/gradient_strokes_and_order.rs` (149) — 4 tests: gradient_offsets_out_of_order_are_clamped_not_sorted, a_gradients_stroke_stays_a_flat_colour, a_gradient_on_a_stroke_only_kind_is_ignored, a_gradient_inside_a_group_moves_with_the_group
- `tests/canvas/rt_canvas_rasteriser/gradient_transforms.rs` (141) — 4 tests: a_linear_gradient_turned_by_its_transform_runs_top_to_bottom, a_linear_gradient_on_a_rotated_item_rotates_with_it, spread_pads_reflects_and_repeats_past_the_end, a_radial_gradient_scaled_two_to_one_is_an_ellipse
- `tests/canvas/rt_canvas_rasteriser/gradient_focal.rs` (171) — 5 tests: a_focal_offset_moves_the_first_stop_to_the_focus, a_focal_offset_beyond_the_circle_is_clamped, a_focal_radius_fills_a_disc_with_the_first_stop, fill_and_stroke_paints_the_outline_with_the_tinted_ramp, degenerate_gradient_fields_draw_as_their_no_op_forms; local helpers: stroke_ramp
- `tests/canvas/rt_canvas_rasteriser/group_presents.rs` (145) — 4 tests: a_group_replaced_between_two_identical_presents_redraws_with_the_new_contents, three_identical_presents_of_an_unchanged_group_draw_one_frame, set_group_deep_copies_and_remove_group_frees_nothing_yet, present_layers_sees_a_group_replaced_under_the_same_name
- `tests/canvas/rt_canvas_rasteriser/group_rendering.rs` (225) — 6 tests: a_group_renders_at_its_offset_nested_diamond_and_absent, a_group_cycle_and_an_over_deep_chain_both_raise, a_clip_inside_a_translated_group_moves_with_it, removing_a_group_a_parent_names_makes_the_parents_node_a_no_op, a_group_offset_far_off_surface_draws_nothing_and_does_not_raise, a_polygon_inside_a_translated_group_draws_at_the_offset
- `tests/canvas/rt_canvas_rasteriser/group_retirement.rs` (205) — 4 tests: a_removed_groups_buffer_is_retired_not_freed, the_group_drain_does_not_depend_on_the_scene_changing, replacing_a_group_frees_only_the_displaced_buffer, scene_draws_shares_one_base_between_a_diamonds_two_draws
- `tests/canvas/rt_canvas_rasteriser/resource_lifetime_mid_frame.rs` (196) — 5 tests: removing_a_group_mid_frame_lets_the_frame_finish, exiting_while_a_frame_draws_a_group_is_clean, installing_and_removing_many_named_groups_does_not_grow_without_bound, an_image_destroyed_while_a_scene_names_it_renders_a_frame_and_does_not_raise, destroying_a_font_mid_frame_is_clean
- `tests/canvas/rt_canvas_rasteriser/ellipse_newton_solvers.rs` (104; no `use super::*;`, see §3) — the four fns nested inside `measure_the_ellipse_newton_iteration_count`, hoisted verbatim with their doc comments: `truth` (37), `solve` (34), `bisect` (28), `steps` (5) (the map's spans L2578-2624, L2626-2666, L2668-2706, L2708-2712 include the doc comments; `grep -n '^    fn ' tests/canvas/rt_canvas_rasteriser.rs` → `truth` 2583, `solve` 2627, `bisect` 2678, `steps` 2708), each `pub(super)`. They reference no parent item (their only non-local names are `std::f64::consts::FRAC_PI_2` by full path, `f64::INFINITY`, and `truth`'s own nested `const COARSE`), so the file has no `use super::*;`.
  `ellipse_newton_measurement.rs` measures 157, not 260: 259 − 104 + `use super::ellipse_newton_solvers::{bisect, solve, steps, truth};` as the first statement of the fn body + `use super::*;`. Nested fn items cannot capture locals, so hoisting is pure motion (decided, §Decisions). The root declares this support module too: 28 `#[path]` pairs, root 188.
Surface: root declares 27 topic + 1 support module(s) (`ellipse_newton_solvers`, reached by the fn-local `use`, so the root adds no glob for it) as `#[path = "rt_canvas_rasteriser/<name>.rs"] mod <name>;`; target name `rt_canvas_rasteriser` unchanged. none for root/topic items (private root items are visible to child modules; topics start with `use super::*;`).
Sweeps: citations: `src/docs/spec/app/06_canvas.md:266` `path_placement_a_clipped_polygon_moves_whole_with_its_position` → `tests/canvas/rt_canvas_rasteriser/path_placement.rs`; `src/docs/spec/app/06_canvas.md:1500` `group_affine_a_nested_group_composes_with_its_parent` → `tests/canvas/rt_canvas_rasteriser/group_affine.rs`; `src/docs/spec/app/06_canvas.md:1512` `group_affine_a_scaled_group_scales_its_strokes` → `tests/canvas/rt_canvas_rasteriser/group_affine.rs` | include_str!/include_bytes! in moved code: none | **`measure_the_ellipse_newton_iteration_count` (#[ignore], L2554-2903) is one 259-LOC fn** — decomposed by hoisting its four nested fns into `ellipse_newton_solvers.rs` (above); `measure_the_transformed_distance_correction` (#[ignore], 84) gets its own file so both design measurements stay `cargo test --test rt_canvas_rasteriser -- --ignored` runnable (filter unchanged).

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
then the phase's sweeps, then K2–K5 (and K6 where the phase edits citations), then commit the new
files, the roots, the swept docs and the ceiling file together. All runs happen on the macOS
development host, which is the only host with Metal.

### Phase 1 — Small canvas suites (§4.1–§4.7)

Seven files with no citation, no include and at most one support module.
`T="rt_canvas_frame_stats rt_canvas_picture_fit rt_canvas_present_deep_copy rt_canvas_present_leak rt_canvas_tile rt_canvas_hit rt_canvas_picture"`,
`RE="rt_canvas_(frame_stats|picture_fit|present_deep_copy|present_leak|tile|hit|picture)"`.

- [ ] K1 → 72 tests.
- [ ] `rt_canvas_frame_stats.rs` → root keeps 7 tests + `wait_frame.rs` (§4.1).
- [ ] `rt_canvas_picture_fit.rs` → root keeps all 9 tests + support `driver.rs` (`pub(super)` chunk, DRIVER, cases, value; root `use driver::*;`) (§4.2).
- [ ] `rt_canvas_present_deep_copy.rs` → root keeps 2 tests + `publish_order.rs`, `groups.rs` (§4.3).
- [ ] `rt_canvas_present_leak.rs` → root (keeps `#![cfg(unix)]` and 4 tests) + `group_animation.rs` (§4.4).
- [ ] `rt_canvas_tile.rs` → root keeps 2 tests + `viewport_and_repaint.rs` (§4.5).
- [ ] `rt_canvas_hit.rs` → root (keeps `#![cfg(unix)]` and `FIXTURE_TTF`, which `bugs/bug-1091-…:41` cites) + `hit_region.rs`, `hit_at_and_id_path.rs`, `over_id_and_events.rs` (§4.6).
- [ ] `rt_canvas_picture.rs` → root + `drawing.rs`, `set_bytes.rs`, `shaped.rs`, `clip_group_and_destroy.rs` (§4.7). The topic module `shaped` and the root helper fn `shaped` live in different namespaces (type and value), so they do not collide.
- [ ] Tighten: K5 and commit the ceiling file.

Acceptance: 72 tests are listed unchanged and pass, and the seven paths leave the banner.
  Check: K2 → no output; K3 → 7 × ok; K4 → no output; K5 → 7 TIGHTEN rows, then none (est. 12 min).
Commit: —

### Phase 2 — Autopaint, damage, graphics thread, group ownership, image decode (§4.8–§4.12)

Five files, one spec citation, and one scene include.
`T="rt_canvas_autopaint rt_canvas_damage rt_canvas_graphics_thread rt_canvas_group_ownership rt_canvas_image_decode"`,
`RE="rt_canvas_(autopaint|damage|graphics_thread|group_ownership|image_decode)"`.

- [ ] K1 → 86 tests.
- [ ] `rt_canvas_autopaint.rs` → root + support `state_scene.rs` (16 `pub(super)` items) + `bare_shape_and_cost.rs`, `curves.rs`, `redraw_and_pictures.rs`, `states.rs` (§4.8).
- [ ] `rt_canvas_damage.rs` → root + `unchanged_and_moved.rs`, `transformed_boxes.rs`, `grouped_boxes.rs`, `resize.rs`, `groups.rs` (§4.9).
- [ ] `src/docs/spec/app/06_canvas.md:1548`: `[[tests/canvas/rt_canvas_damage.rs:turning_a_group_repaints_and_matches_a_full_redraw]]`
      → `[[tests/canvas/rt_canvas_damage/groups.rs:turning_a_group_repaints_and_matches_a_full_redraw]]`.
- [ ] `rt_canvas_graphics_thread.rs` → root + `metal.rs`, `image_lifetime.rs`, `autopaint_windows.rs`, `frame_pacing.rs` (§4.10). In `frame_pacing.rs`, `include_str!("scenes/frame_skip_nested_lists.mfb")` → `include_str!("../scenes/frame_skip_nested_lists.mfb")`.
- [ ] `rt_canvas_group_ownership.rs` → root keeps 2 tests + `mid_frame.rs`, `appended.rs` (bug-1092's four tests), `binding.rs` (§4.11).
- [ ] `rt_canvas_image_decode.rs` → root + support `png_builder.rs` (11 `pub(super)` items) + `pixel_formats.rs`, `inflate_and_refusals.rs`, `size_caps.rs` (§4.12).
- [ ] Tighten: K5 and commit the ceiling file and `06_canvas.md`.

Acceptance: 86 tests are unchanged and pass, and the damage citation names the file that holds the test.
  Check: K2 → no output; K3 → 5 × ok; K4 → no output; K5 → 5 TIGHTEN rows, then none; K6 → ok and
  `grep -n 'fn turning_a_group_repaints_and_matches_a_full_redraw' tests/canvas/rt_canvas_damage/groups.rs` → 1 hit (est. 15 min).
Commit: —

### Phase 3 — The GPU golden and Metal suites (§4.13–§4.14)

Two files with 57 scene includes, two citations, the Vulkan `sed` pin, and the stale
`.ai/testing-gates.md` path.
`T="rt_canvas_golden rt_canvas_metal"`, `RE="rt_canvas_(golden|metal)"`.

- [ ] K1 → 99 tests. Record `for c in GROUPS POLYLINES; do sed -n "/^const $c: &str = r#\"/,/^\"#;\$/p" tests/canvas/rt_canvas_golden.rs | wc -l; done` → `26`, `43`.
- [ ] `rt_canvas_golden.rs` → root (keeps `GROUPS`, `POLYLINES` unindented, plus the render helpers) + eight topics (§4.13). The 4 scene includes become `"../scenes/…"`.
- [ ] `06_canvas.md:1592`: `[[tests/canvas/rt_canvas_golden.rs:the_gpu_draws_the_group_affine_scene_the_reference_shows]]` → `[[tests/canvas/rt_canvas_golden/groups.rs:…]]` (same symbol).
- [ ] `rt_canvas_metal.rs` → root + nine topics (§4.14). The 53 scene includes become `"../scenes/…"` exactly as listed in §4.14.
- [ ] `06_canvas.md:1593`: `[[tests/canvas/rt_canvas_metal.rs:a_group_with_a_matrix_draws_on_metal_and_matches_the_software_oracle]]` → `[[tests/canvas/rt_canvas_metal/fallback_and_transforms.rs:…]]`.
- [ ] `.ai/testing-gates.md:314`: `tests/rt_canvas_metal.rs` → `tests/canvas/rt_canvas_metal.rs`. This is a pre-existing stale path (evidence: `ls tests/rt_canvas_metal.rs` → No such file).
- [ ] Include sweep: `grep -rn 'include_str!("scenes/' tests/canvas/rt_canvas_golden tests/canvas/rt_canvas_metal tests/canvas/rt_canvas_graphics_thread` → no output.
- [ ] Tighten: K5 and commit the ceiling file, `06_canvas.md` and `.ai/testing-gates.md`.

Acceptance: 99 tests are unchanged and pass on Metal, both citations resolve to their symbol, and
the Vulkan script still extracts its scenes.
  Check: K2 → no output; K3 → 2 × ok; K4 → no output; K5 → 2 TIGHTEN rows, then none; K6 → ok,
  and `grep -n 'fn the_gpu_draws_the_group_affine_scene_the_reference_shows' tests/canvas/rt_canvas_golden/groups.rs`
  and `grep -n 'fn a_group_with_a_matrix_draws_on_metal_and_matches_the_software_oracle' tests/canvas/rt_canvas_metal/fallback_and_transforms.rs`
  → 1 hit each; the `sed` line counts are again `26`, `43`; `grep -n 'tests/rt_canvas' .ai/testing-gates.md` → no output (est. 15 min).
Commit: —

### Phase 4 — Font, curves and particles (§4.15–§4.17)

Three files: the font fixture builder moves (so its prose references are repointed), and the
`MODEL_DRIVER` extraction.
`T="rt_canvas_font rt_canvas_curves rt_canvas_particles"`, `RE="rt_canvas_(font|curves|particles)"`.

- [ ] K1 → 102 tests.
- [ ] `rt_canvas_font.rs` → root + support `fixture.rs` (no `use super::*;`) and `render.rs` + nine topics (§4.15).
- [ ] Fixture-builder references → `tests/canvas/rt_canvas_font/fixture.rs`:
      `scripts/remote-common.sh:47` and `scripts/test-canvas-vulkan.sh:88` (both say
      `tests/canvas/rt_canvas_font.rs`); `scripts/man-run-examples.sh:260` (says
      `tests/rt_canvas_font.rs`, pre-existing stale: `ls tests/rt_canvas_font.rs` → No such file);
      `planning/completed/plan-162-B-kerning.md:59` (`tests/canvas/rt_canvas_font.rs:truetype_fixture`) and
      `:164` (the fixture-builder task).
- [ ] `bugs/bug-1091-every-text-geometry-build-copies-the-whole-font-file.md:130`: "Add a test to
      `tests/canvas/rt_canvas_font.rs`" → "Add a test to `tests/canvas/rt_canvas_font/transforms_and_cache.rs`",
      the topic that holds the sibling glyph-cache test.
- [ ] Reference sweep: `grep -rn 'rt_canvas_font\.rs' scripts planning/completed/plan-162-B-kerning.md bugs/bug-1091-*.md`
      → no output (target-name mentions such as `cargo test --test rt_canvas_font` carry no `.rs`).
- [ ] `rt_canvas_curves.rs` → root + support `driver.rs`, `curve_model.rs`, `scene.rs` (both `impl Scene` blocks), `nurbs_model.rs` + eleven topics (§4.16).
- [ ] `rt_canvas_particles.rs` → root + support `render.rs` + five topics (§4.17), and the
      extraction: write the bytes between `r#"` (line 256) and `"#` (line 505), that is from line
      256's newline through line 504's `END FUNC\n`, verbatim to
      `tests/canvas/rt_canvas_particles/model_driver.mfb`. In `model.rs`, declare
      `const MODEL_DRIVER: &str = include_str!("model_driver.mfb");`.
- [ ] Byte check of the extraction:
      `git show HEAD:tests/canvas/rt_canvas_particles.rs | python3 -I -c 'import sys; t=sys.stdin.read(); a="const MODEL_DRIVER: &str = r#\""; s=t.index(a)+len(a); sys.stdout.write(t[s:t.index("\"#;", s)])' | cmp - tests/canvas/rt_canvas_particles/model_driver.mfb`
      → no output (`HEAD` is the tree before this phase).
- [ ] `python3 -I tools/loc-census/census.py --file tests/canvas/rt_canvas_particles/model_driver.mfb` → `0 216`.
- [ ] Tighten: K5 and commit the ceiling file, the three scripts, `plan-162-B-kerning.md` and `bug-1091-…md`.

Acceptance: 102 tests are unchanged and pass, the driver string is byte-identical, and every live
reference to the fixture builder names the file that holds it.
  Check: K2 → no output; K3 → 3 × ok; K4 → no output; K5 → 3 TIGHTEN rows, then none; the `cmp`
  → no output (est. 15 min).
Commit: —

### Phase 5 — rt_canvas_rasteriser (§4.18)

The largest split: 109 tests in 27 topics, the nested-fn hoist, and three citations.
`T="rt_canvas_rasteriser"`, `RE="rt_canvas_(rasteriser)"`.

- [ ] K1 → 109 tests (including the two `#[ignore]`d measurements).
- [ ] `rt_canvas_rasteriser.rs` → root (188) + 27 topics + `ellipse_newton_solvers.rs` (§4.18).
- [ ] Hoist: cut `truth`, `solve`, `bisect` and `steps` (with their `///` docs) out of
      `measure_the_ellipse_newton_iteration_count` into `ellipse_newton_solvers.rs` as
      `pub(super) fn`. Bodies are unchanged, and `truth` keeps its nested `const COARSE`. Make
      `use super::ellipse_newton_solvers::{bisect, solve, steps, truth};` the first statement of the
      test body. `measure_the_transformed_distance_correction` gets its own topic,
      `distance_correction_measurement.rs`.
- [ ] Citations in `06_canvas.md`: `:266` `[[tests/canvas/rt_canvas_rasteriser.rs:path_placement_a_clipped_polygon_moves_whole_with_its_position]]`
      → `[[tests/canvas/rt_canvas_rasteriser/path_placement.rs:…]]`; `:1500` and `:1512`
      (`group_affine_a_nested_group_composes_with_its_parent`, `group_affine_a_scaled_group_scales_its_strokes`)
      → `[[tests/canvas/rt_canvas_rasteriser/group_affine.rs:…]]`.
- [ ] Declaration check: `grep -c '^#\[path = "rt_canvas_rasteriser/' tests/canvas/rt_canvas_rasteriser.rs` → `28`;
      `ls tests/canvas/rt_canvas_rasteriser/*.rs | wc -l` → `28`.
- [ ] Run the two design measurements once, since only they exercise the hoisted fns:
      `cargo test --release --test rt_canvas_rasteriser -- --ignored` → ok.
- [ ] Tighten: K5 and commit the ceiling file and `06_canvas.md`.

Acceptance: 109 tests are unchanged and pass, both ignored measurements run, and the three
citations resolve to their symbols.
  Check: K2 → no output; K3 → ok; the `--ignored` run → ok; K4 → no output; K5 → 1 TIGHTEN row,
  then none; K6 → ok and each of the three `grep -n 'fn <symbol>' <new path>` → 1 hit (est. 20 min:
  the release `--ignored` run is the only execution of the hoisted solvers).
Commit: —

## Validation Plan

- Tests: none are added. Every moved test runs in its phase's K3. The ignored rasteriser
  measurements run in Phase 5.
- Coverage check: `tests/` is outside the coverage denominator (`scripts/coverage-common.sh`
  IGNORE), and no compiler or runtime code changes.
- Runtime proof: each phase's K3 on the macOS Metal host. The Linux Vulkan script's inputs (the
  `GROUPS`/`POLYLINES` `sed` extraction) are checked in Phase 3 without a Linux run, because the
  extracted text is unchanged.
- Doc sync: `src/docs/spec/app/06_canvas.md` (six citations), `.ai/testing-gates.md:314`,
  three `scripts/` comments, `planning/completed/plan-162-B-kerning.md`, and
  `bugs/bug-1091-every-text-geometry-build-copies-the-whole-font-file.md`. No `mfb spec` contract
  changes, only where its evidence lives. Citation count check:
  `grep -c '\[\[tests/canvas/' src/docs/spec/app/06_canvas.md` → `6` before Phase 2 and after Phase 5.
- Final gate (once, after Phase 5):
  `cargo test --test file_size_budget 2>&1 | grep -E 'tests/canvas/rt_canvas_[a-z_]+(\.rs|/)'` → no output;
  `grep -E '^(ceiling|exempt) .*tests/canvas/rt_canvas_' tests/guards/file_size_ceilings.txt` → no output;
  `cargo test -p mfb --bin mfb spec_citations_resolve` and `cargo test --test test_targets_registered` → ok
  (est. 4 min). The full workspace suite runs once, in plan-176-V.

## Decisions

- Rule: code ≤ 1000, test ≤ 250 non-comment, non-blank LOC, over `.rs`/`.mfb` under `src/`,
  `packages/`, `examples/`, `repository/` and `tests/` — owner, 2026-10-14.
- Inline test code counts against 250; generated files are exempt; the guard warns and never fails;
  a new module is always an acceptable split — owner, 2026-10-14.
- A single test-side unit over 250 is exempted only after the natural extractions are done
  (raw-string program → `include_str!` `.mfb`; hoisting nested helper fns) — owner, 2026-10-14.
  For D, both extractions apply, and no D file is exempt.
- `rt_canvas_particles`' `MODEL_DRIVER` moves to `tests/canvas/rt_canvas_particles/model_driver.mfb`
  via `include_str!` — author, 2026-10-14 (NOT exempt).
- `measure_the_ellipse_newton_iteration_count`'s nested fns are hoisted into
  `rt_canvas_rasteriser/ellipse_newton_solvers.rs` and imported by a fn-local `use` — author,
  2026-10-14 (NOT exempt).
- Data-only support files omit `use super::*;` (an unused glob warns) — author, 2026-10-14.
- Glob-cycle imports compile without warnings (`/tmp/loc/probe2` printed `6`, no warning) — verified.
- Pre-existing stale references found while sweeping are fixed in the phase that touches the file:
  `.ai/testing-gates.md:314` and `scripts/man-run-examples.sh:260` — AGENTS.md "never leave a bug".
- Live docs (scripts, open plans, open bugs) are repointed when a symbol they name moves. Archived
  docs under `*/completed/` are not edited — author, 2026-10-14.

## Corrections

## Summary

The risk is a topic that is never compiled (the 27 rasteriser topics), a scene include that misses
(this is a compile error, so it is loud), and the one string extraction (closed by a byte `cmp`).
The spec citation sweep is manual, because the resolver is file-level. K6's symbol grep makes it
checkable. Nothing outside `tests/canvas/`, the ceiling file, six citation lines, two `.ai` and three
script comment lines, and three lines in two live planning documents changes.

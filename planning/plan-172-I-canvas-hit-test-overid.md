# plan-172-I: Hit-testing and `MouseEvent.overID`

Last updated: 2026-10-11
Effort: large (3h–1d)
Depends on: plan-172-H

The canvas gains one hit test — "which drawn item is topmost under this point" — and
its first user: `canvas::MouseEvent.overID`, the id path of that item. `canvas::Group`
gains an `id`, so each use of a group names what it draws. 172-K reuses the same
function on the graphics thread for `AutoPaint`.

Behavioral outcome: a program gives a shape an `id`, enables the mouse, and each event
from `canvas::pollMouse` names the shape under the pointer, tested against the shape
itself — after every transform — not its bounding box. A group drawn twice under two
`id`s reports which of the two was hit.

References:

- `planning/new_canvas.md` — "Hit-testing", "Groups and identity" (the id path, as
  rewritten on 2026-10-10), the `Group` listing and the `MouseEvent` listing with its
  two `overID` rules.
- `planning/plan-172-A-canvas-shapes-premises.md` §4 P4 and P5, Open Decision 5.
- `src/codegen/builtins/canvas/helper_draw.rs:__canvas_geoDistance` — the distance
  function the test is built on.
- `src/codegen/builtins/canvas/helper_render.rs:__canvas_appendDraw` — the walk that
  flattens groups into draw entries.
- `src/codegen/builtins/canvas/func_poll_mouse.rs`, `src/codegen/io/mouse/ring.rs`.
- `.ai/canvas-threading.md` §2 — module-level globals, the geometry cache among them,
  are per-thread.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-H complete and archived | `ls planning/plan-172-H-*` → no matches | MET — `no matches found`, 2026-10-11 |
| Premises P4 and P5 have verdicts | `grep -cE 'P[45].*(CONFIRMED\|FALSIFIED)' planning/completed/plan-172-A-*.md` → 2 | MET — prints 3, not 2: both verdict rows and one sentence that names both (Corrections) |
| Every shape has an `id` | `grep -c 'id_prop()' src/codegen/builtins/canvas/mod.rs` → ≥ 9 | MET — 10 before this letter, 11 after (`Group`) |

If plan-172-H is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `__canvas_hitAt(x, y)` returns the topmost draw entry containing a surface point, or
  −1, over whichever draw list the calling thread built.
- The hit region follows `planning/new_canvas.md`:
  - a closed shape is hit on its interior and its stroke, filled or not;
  - `Line`, `PolyLine` and `Arc` are hit on the stroke, at least 3 pixels either side;
  - `Text` is hit on its layout box;
  - the shape's own `paint.clip` cuts the region;
  - every shape and group transform is applied before the test.
- `canvas::Group` is `id`, `position`, `transform`, `name`.
- `canvas::MouseEvent` has `overID AS String`, last.
- `overID` is the **id path** of the topmost item under `position` in the scene the
  program last presented: the `id` of every group that holds it, outermost first, then
  its own, joined by `::` (Open Decision 1), with empty parts left out. So:
  - a named shape in the scene itself reports its `id`, as a scene with no named group
    always has;
  - a shape `"label"` inside a group with `id` `"save"` reports `"save::label"`; nested
    groups chain, `"toolbar::save::label"`;
  - an unnamed shape inside a named group reports the group's path alone;
  - with no `id` anywhere on the path, or no item under the point, it is `""`.
- A `Picture` contributes its `id` as the last part today. 172-L gives it a shape, whose
  `id` then follows the picture's.
- A program in which no item has an `id` pays nothing for any of this.

### Non-goals (explicit constraints)

- **No `AutoPaint`, no pointer state on the graphics thread, no new redraw trigger.**
  Those are 172-J and 172-K.
- **`overID` is not resolved at the moment the event happened.** It is resolved when
  `pollMouse` returns the event.
- **A `Picture`'s shape is 172-L.** Here a picture is one named item.
- **The path is not parsed back.** `overID` is one string; no function splits it, and a
  program that puts `::` inside an `id` gets what it wrote.
- **`AutoPaint` gets no `id`** (172-K): it is a decorated shape and reports its shape's.
- **No path of indices.** Two uses of one group with the same `id`, or with none, report
  the same string for the same item, as they did before groups had one.
- **`term::pollMouse` and `term::MouseEvent` are untouched.**
- **Hit-testing is not exposed as a public function** in this plan.

## 2. Current State

`canvas::pollMouse` is a native function that dequeues one decoded event from the
worker's ring and builds a 64-byte `MouseEvent` block by hand: six 8-byte slots and an
inlined two-float position. It reads no scene.

The distance functions are MFBASIC: `__canvas_geoDistance` takes a kind, the geometry
tail and a shape-space point; the caller applies the inverse transform and the distance
rescale. They read the geometry cache through `__canvas_geoAt`. That cache is a
module-level global, so each thread has its own; only the graphics thread fills one
today.

The worker reads its last-presented scene through `canvas::installedItems()` or
`installedLayers()`, each a deep copy.

### Measured populations

| What | Count | Command |
|---|---|---|
| `canvas::pollMouse` mentions | 23 in 13 files | `grep -rho 'canvas::pollMouse' src tests examples tools scripts benchmark \| wc -l` → 23 |
| Canvas runtime tests that poll the mouse | 0 | `grep -rl 'canvas::pollMouse' tests/canvas \| wc -l` → 0 |
| `MouseEvent` block size | 64 bytes | `grep -n 'const MOUSE_EVENT_BLOCK_BYTES' src/codegen/builtins/canvas/func_poll_mouse.rs` → `= 64` |
| `canvas::Group[` sites this letter ports (`id` added, empty) | 118 in 23 files | `grep -rhoE 'canvas::Group\[' src tests examples tools scripts benchmark \| wc -l` → 118, measured 2026-10-10 after 172-H |
| Geometry kinds a hit test must handle | 10 (0–4, 6, 7, 9, 10; `NONE` and `GROUP` never reach a draw entry) | `grep -n '__CANVAS_GEO_[A-Z]* AS Integer\|__CANVAS_KIND_[A-Z]* AS Integer' src/codegen/builtins/canvas/helper_geometry.rs src/codegen/builtins/canvas/helper_draw.rs` |
| Cost of a hit test on 1,000 and 10,000 items | UNMEASURED | Phase 4 measures it; no cap or cache is designed against a guess |

### Verified properties

- **Injected mouse events reach `pollMouse` in a headless run.** VERIFIED by reading
  `src/codegen/term/core/term.rs:emit_mouse_inject`: `MFB_MOUSE_INJECT` feeds raw SGR
  bytes through the real decoder at `enableMouse(TRUE)`. POSIX only.
- **Nothing in `tests/canvas` exercises `canvas::pollMouse`.** VERIFIED by the grep
  above. This letter adds the first runtime test of it.
- **The outcome of P4 decides where geometry comes from** on the worker; this section
  is written for P4 CONFIRMED and §3 names the change if it was not.

## 3. Design Overview

**One test, two callers.** `__canvas_hitAt(x, y)` walks the calling thread's current
draw list from the last entry to the first. For each entry it:

1. rejects on the entry's world bounds;
2. maps the point through the entry's group matrix and the item's own inverse;
3. applies the item's clip, if any;
4. tests the shape: `distance ≤ strokeHalf` for a closed kind; `|distance| ≤
   max(strokeHalf, 3 / scale)` for an open kind; point-in-box for text.

The first entry that passes is the answer. The walk appends each entry's id path to a
parallel global, `__CANVAS_DRAW_IDS`, so the index maps to a string.

**The id path is built by the walk.** `__canvas_appendDraw` already carries a group's
composed translation and matrix down to its children (172-G); it carries the path the
same way. The group arm passes `path` joined with the node's `id`, and a leaf records
`path` joined with its own. Joining leaves an empty part out, so a scene with no named
group records exactly each item's `id`. The path is a property of the draw ENTRY, like
the offset: the group's stored items are not copied or renamed, and one item reached
through two nodes has two entries with two paths.

A group's `id` does not enter the draw hash or the geometry: it changes no pixel.
`present` compares the scene's bytes, which include it, so a node whose `id` changes is
republished and the worker's draw list is rebuilt.

**The worker's draw list.** The worker builds its own, with the same
`__canvas_appendDraw` walk over its installed scene, into its own thread-local geometry
cache. It is built lazily: only when a real event is being returned, only when the
scene revision has changed since the last build, and only when the published scene has
at least one non-empty `id` — on an item or on a group node, at any depth, the items of
the groups it names included. `present` records that last fact as it hashes the scene
and takes the group signature.

If P4 was FALSIFIED, the worker's walk builds each geometry record into a list it owns
and the distance readers take that list as an argument; the test itself is unchanged.

**`pollMouse` becomes a wrapper** (P5). The native function is renamed
`canvas::pollMouseRaw`, `internal_only`, returning the six fields it returns today in an
internal record. Public `pollMouse` is MFBASIC: it calls the raw poll and, for any event
whose kind is not `None`, builds a `MouseEvent` by name with `overID` from
`__canvas_hitAt`. The `canvas::setBytes` over `setBytesRaw` pair is the precedent.

**Gate class: new behaviour.** No existing pixel test is affected: nothing on the
graphics thread changes.

**Where the risk is:** cost. A scene that changes every frame, with the pointer moving,
rebuilds the worker's geometry every frame. Phase 4 measures that before the letter
closes, and the id-less fast path keeps every existing program at zero.

### Rejected alternatives

- **Resolving `overID` on the graphics thread and reporting the last frame's answer.**
  Recorded as the alternative in 172-A Open Decision 5. It avoids the second geometry
  cache but reports an item for a position the event did not have.
- **Bounding-box hit-testing.** Rejected by `planning/new_canvas.md`.
- **A public `canvas::hitTest`.** Not asked for; adding public surface is a separate
  decision.

## Compatibility / Format Impact

- **Added:** `MouseEvent.overID`; `Group.id`.
- **Changed:** `canvas::Group`'s fields — `id` is first, as on every other item. Every
  `canvas::Group[` constructor that names its fields gains `id := ""`; one that uses
  `MUT` + `WITH` needs no edit.
- **Changed:** `canvas::pollMouse` is MFBASIC over a native raw poll; the `MouseEvent`
  block is laid out by the type model rather than by hand.
- **Unchanged:** the mouse ring, the SGR producers on all three platforms, every pixel.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — The point test, per kind, as pure functions

- [x] New `src/codegen/builtins/canvas/helper_hit.rs`: `__canvas_hitEntry(offset,
      matrix, x, y) AS Boolean`, built on `__canvas_geoDistance`, the clip path
      distance, and a box test for text. Register it beside the other `helper_*` files.
      → `__canvas_hitEntry(offset, mat, gdx, gdy, x, y)` — two more parameters than
      written, the entry's group translation (Corrections). Registered in `mod.rs` as
      ONE chunk gated on `pollMouse` being called, not as an always-compiled helper
      (Corrections).
- [x] Unit cases beside it, one per kind: a point inside, on the stroke, 2 pixels off a
      1-pixel line (hit), 4 pixels off (miss), inside an unfilled rectangle (hit),
      inside a clipped-away part (miss), inside a rotated ellipse's corner box but
      outside the ellipse (miss).
      → They are runtime cases in `tests/canvas/rt_canvas_hit.rs`, not unit cases
      beside the helper: the in-binary canvas tests can only read source text
      (Corrections). `hit_region_a_closed_shape_is_its_interior_and_its_stroke` (10
      points: inside, on the stroke, an unfilled and a zero-paint rectangle hit, a
      circle's bounding-box corner missed, a picture);
      `hit_region_an_open_shape_is_its_stroke_with_three_pixels_of_slack` (16 points:
      2 and 3 px off a 1 px line hit, 4 px missed, a 20 px line with no slack, an arc
      off its sweep, a polyline's inside corner missed);
      `hit_region_text_is_its_layout_box` (8 points, filled and stroked text);
      `hit_region_follows_the_clip_and_the_transform` (9 points: a rectangle clip, a
      path clip, the rotated ellipse's corner-box miss, a scaled line's slack in
      surface pixels).

Acceptance: every per-kind case passes, including the bounding-box-but-not-shape miss.
  Check: `cargo test --bin mfb canvas_hit` → pass (est. 4 min).
  → `cargo test --release --test rt_canvas_hit hit_region` → `4 passed; 0 failed`
  (87.5 s, most of it the build); `cargo test --bin mfb canvas_hit` → `2 passed; 0 failed` (the
  two text tests that file does hold).
Commit: —

### Phase 2 — The draw-list walk

- [x] `mod.rs`: `Group` gains `id` first (`id_prop()`), and its description says what
      the `id` is for; `helper_render.rs:__canvas_noItem` and the second stand-in node
      in `__canvas_hashLayers` take the field.
      → Done. The second stand-in is in `__canvas_hashScene`, not `__canvas_hashLayers`
      (which builds none). `id_prop()`'s own description now says `overID` reports it.
- [x] Port the `canvas::Group[` sites: `id := ""` first. Re-measure the count first
      (118 on 2026-10-10) and record it.
      → `grep -rhoE 'canvas::Group\[' src tests examples tools scripts benchmark | wc
      -l` → 118 before the port, as measured. `/tmp/plan-172-A/codemod_i.py` ported
      120 constructors in 24 files: those 118 and the two internal `Group[` stand-ins.
      46 of them sit inside escaped Rust string literals and needed `id := \"\"`
      (`codemod_i2.py`: `escaped 46 left bare 36`) — the first test round failed to
      COMPILE four test targets on that (Corrections).
- [x] `FIELD_KIND_TABLE` and the `canvas::Group` row of `field_kinds.tsv` (its check
      column follows the generator); the IR-embedding goldens.
      → `canvas::Group`'s check column is `before.id`. `canvas::MouseEvent` moved too,
      which the plan did not list: a `String` makes it a size-varying record, so its
      `FIELD_KIND_TABLE` entry is `SIZE_VARIES` and its row takes the expectations
      every size-varying record has (Corrections). Two rows edited by hand; the table
      is not the generator's output (`field_kinds.tsv`'s own header: "a letter that
      lands a kind flips its sites by hand"). `MFB_SELF_UPDATE_FILTER='canvas::Group|canvas::MouseEvent' cargo test --test
      rt_inplace_self_update every_field_kind_meets_its_expectation -- --ignored` →
      `1 passed; 0 failed` (130 s); `cargo test --test inplace_self_update_census` →
      `2 passed`.
- [x] `helper_render.rs:__canvas_appendDraw`: take the id path as a parameter, join a
      group node's `id` onto it for its children, and append each entry's path joined
      with the item's own `id` to `__CANVAS_DRAW_IDS`; reset it wherever the offset
      arrays are reset. One `__canvas_joinId(path, id)` does the joining, with `::`,
      and leaves an empty part out.
      → NOT as written: `__canvas_appendDraw` is untouched. The path is carried by a
      second walk, `helper_hit.rs:__canvas_hitAppend`, into `__CANVAS_HIT_IDS`
      (Corrections). `__canvas_joinId` is as written.
- [x] `helper_hit.rs`: `__canvas_hitAt(x, y) AS Integer`, last entry first.
- [x] Runtime cases in new `tests/canvas/rt_canvas_hit.rs`, driven through a
      debug-build probe function the way `MFB_CANVAS_STATS` reads are: two overlapping
      shapes report the upper; an item inside a rotated, scaled group is hit at its
      transformed place and missed at its untransformed one; an item in a layer above
      wins over one in a layer below.
      → No probe: every case asks the public question, a press injected with
      `MFB_MOUSE_INJECT` and `overID` read back from `canvas::pollMouse` (Corrections).
      `hit_at_the_upper_of_two_overlapping_items_answers`,
      `hit_at_an_item_in_a_transformed_group_is_at_its_transformed_place`,
      `hit_at_an_item_in_a_layer_above_wins`.
- [x] Id-path cases in the same file, read through the same probe: one group drawn
      through two nodes with different `id`s reports two different paths for the same
      item; an unnamed item inside a named group reports the group's `id` alone; a named
      item inside an unnamed group reports its own `id` alone; a group inside a group
      reports both, outermost first; nothing named on the path reports `""`.
      → `id_path_names_every_group_that_holds_the_item`: `save::face` and
      `quit::face`, `save`, `leaf`, `bar::inner::face`, `""`.

Acceptance: the three hit cases and the five id-path cases pass, and every group test
passes with no PNG regenerated.
  Check: `cargo test --test rt_canvas_hit --test rt_canvas_group_ownership --test
  rt_canvas_golden groups` → pass (est. 8 min); `git status --porcelain
  tests/golden/canvas` → empty.
  → `cargo test --test rt_canvas_hit --test rt_canvas_group_ownership` → `12 passed`
  and `11 passed`; `rt_canvas_golden` → `25 passed` in the full round
  (`/tmp/plan-172-A/canvas_round.sh`: all 25 canvas-embedding test targets green, with
  `--bin mfb canvas` 117 passed and `--bin mfb spec` 46 passed); `git status --porcelain
  tests/golden/canvas | wc -l` → 0.
Commit: —

### Phase 3 — `overID`

- [x] `mod.rs`: append `overID AS String` to `MouseEvent`; add the internal raw record.
      → `overID` appended. There is NO raw record: the native poll returns a whole
      `MouseEvent` with `overID` empty (Corrections).
- [x] `func_poll_mouse.rs`: the native becomes `pollMouseRaw`, `internal_only`; add the
      MFBASIC `pollMouse` body and the lazy worker draw list with its three conditions.
      → The native is `canvas::pollMouseRaw() AS MouseEvent`, an 88-byte block it lays
      out by hand. `pollMouse` is `Body::Rewrite("__canvas_pollMouse")`, whose body is
      in `helper_hit.rs`'s gated chunk with `__canvas_overId` and `__canvas_hitBuild`.
      The three conditions: a real event (`__canvas_pollMouse`); the revision moved
      (`__CANVAS_HIT_REV <> __CANVAS_SCENE_REV`); something has an `id`
      (`__CANVAS_SCENE_IDS`, then `__canvas_reachesId` for a scene of unnamed group
      nodes, then `__CANVAS_HIT_NAMED`).
- [x] Rename `canvas.pollMouse` to `canvas.pollMouseRaw` in the six per-target tables
      (`src/target/{macos_aarch64,win_x86_64,linux_common}/{mod,plan}.rs`) and move the
      `src/codegen/registry/mod.rs` record-returning entry (172-A finding 2).
      → Done; `canvas.sceneIdBits` is added beside `canvas.sceneHashes` in the three
      `mod.rs` lists.
- [x] Make the three mouse-state suffix tests (`src/target/shared/plan/symbols.rs:
      module_uses_mouse`, two in `src/codegen/engine/builder/mod.rs`) accept the raw
      poll's symbol, and register the MFBASIC `pollMouse` so a canvas program that never
      calls it does not emit the raw poll or reserve mouse state (172-A findings 3, 4).
      Check: an empty canvas program's executable is the same size before and after
      this phase.
      → `HelperGate::WhenUsed(&["pollMouse"])` on the whole chunk. `sh
      /tmp/plan-172-I/size.sh .` → `base size=3194380`, `new size=3194380`: the same.
      The code plan itself is 540 instructions longer (`ncdiff.sh`: 692151 → 692691,
      in `present`, `presentLayers`, the two stand-in `Group` literals and the new
      47-instruction `sceneIdBits`), which the executable's page padding absorbs. A
      size is a proxy, so the claim it stood for is tested directly:
      `tests/codegen/codegen_mouse_arena_region.rs:
      a_canvas_program_that_never_polls_the_mouse_reserves_no_mouse_state` — no
      `_pollMouseRaw` in the code plan and no `_mfb_rt_mouse_mode` word, and both
      present once the program polls.
- [x] `func_present.rs` / `func_present_layers.rs`: record whether the published scene
      has any non-empty `id` — an item's or a group node's, the items of the groups it
      names included (the walk `__canvas_groupSignature` already makes).
      → NOT in the signature walk. `present` records `__CANVAS_SCENE_IDS =
      canvas::sceneIdBits(items)`, a new native pass, and `__CANVAS_SCENE_REV`; the
      items of the groups a scene names are looked at later and only if needed
      (Corrections).
- [x] Runtime cases in `tests/canvas/rt_canvas_hit.rs` under `MFB_MOUSE_INJECT`: a
      press over a shape with an `id` reports it; over empty canvas reports `""`; over
      an id-less shape lying on an id-bearing one reports `""`; after a `present` that
      moves the shape, the same position reports the new answer; a press over each of
      two uses of one group reports that use's `id` in front of the item's; and a scene
      whose only `id` is on a group node still resolves `overID` (the fast path must not
      skip it).
      → `over_id_follows_the_scene_last_presented` (nothing presented, presented,
      moved, and a scene with no `id`);
      `hit_at_the_upper_of_two_overlapping_items_answers` (the id-less shape on an
      id-bearing one); `id_path_names_every_group_that_holds_the_item` (the two uses);
      `over_id_is_resolved_when_the_only_id_is_on_or_inside_a_group` (the only `id` on
      a node; the only `id` on an item inside an unnamed node's group; and that group
      replaced). Two more than listed: `a_polled_event_is_a_whole_record` (the
      hand-built block is copied into a list and every field read back) and
      `over_id_stays_right_across_many_changed_scenes` (300 changed scenes through the
      worker's cache).
- [x] Regenerate `tests/syntax/app/app-mouse-surface` goldens per
      `.ai/testing-gates.md` and port its source to read `overID`.
      → The fixture reads `e.overID`; its eight goldens moved, and so did
      `loop-body-resource-reference-valid`'s `.ir`, which embeds the canvas source.
      `scripts/artifact-gate.sh target/release/mfb all` → `2322 golden(s) checked, 0
      diff(s)`.
- [x] Port the 23 `pollMouse` mentions where they construct or destructure the record.
      → None constructs a `canvas::MouseEvent` by field (`grep -rn
      'canvas::MouseEvent\[' src tests examples tools scripts benchmark` → the
      field-kind generator's literal and its row, both ported), and reading a field by
      name needs no port. The four example programs that poll build unchanged.
- [x] `src/docs/spec/app/06_canvas.md`: a "Hit-testing" section stating the region
      rules, the id path (its order, the `::` separator, the empty-part rule) and when
      `overID` is resolved, cited to `helper_hit.rs` and `__canvas_appendDraw`; "Named
      groups" gains the node's `id`;
      `src/docs/spec/language/18_builtin-functions.md` where it lists `MouseEvent`.
      → Done; the walk it cites is `__canvas_hitAppend`. `.ai/canvas-threading.md` §14
      gains "The worker has a geometry cache too, once a program polls the mouse".

Acceptance: the four injected-event cases pass.
  Check: `cargo test --test rt_canvas_hit over_id` → pass (est. 4 min).
  → `cargo test --release --test rt_canvas_hit` → `12 passed; 0 failed` (115 s).
Commit: —

### Phase 4 — Cost, measured

- [x] Add a `hit` generator to `tools/canvas-bench/gen/` on the pattern of `worker.sh`:
      N items with ids, the scene changed and one event polled per frame.
      → `gen/hit.sh DIR N IDS` and `bench.sh <mfb> hit`. Sixty presses a frame, not
      one: the clock ticks in microseconds and an id-less poll is tens of nanoseconds,
      so one poll cannot be timed (Corrections).
- [x] Record, on a release build, the per-poll time at 1,000 and 10,000 items, and the
      same with no ids (the fast path), in Corrections.
      → In Corrections, "Phase 4's figures".
- [x] State the two figures in `canvas::pollMouse`'s registry description.
      → `mfb man canvas pollMouse`, "What it costs…": about 0.8 ms and about 9 ms for
      the first event after a changed scene, about 9 µs and about 170 µs for each
      further one, on an Apple M2 Max.
- [x] Update `tools/canvas-bench/README.md` for the new generator.

Acceptance: four measured figures are recorded, and the no-id figure is within noise of
the figure taken at this letter's starting commit.
  Check: `bash tools/canvas-bench/bench.sh target/release/mfb hit` → four figures
  (est. UNMEASURED — time the first run and record it).
  → Four rows, 30.5 s for the first run. The no-id poll is 50 ns median and 33 ns
  minimum at both sizes; the starting commit's, by the same program, is 33 and 33. The
  clock's tick over 59 polls is 17 ns, so the minimums are equal and the medians are
  one tick apart. See Corrections for what it took to get there.
Commit: —

## Validation Plan

- Tests: per-kind unit cases; `tests/canvas/rt_canvas_hit.rs` (new); the regenerated
  `app-mouse-surface` fixture; a syntax fixture for a program reading `overID`.
- Coverage check: confirm `helper_hit.rs` is in the coverage denominator with
  `scripts/coverage-check.sh`; a new file absent from it makes a green gate vacuous.
- Runtime proof: `examples/canvas-demo` with one button given an `id`, run with an
  injected click at its centre, printing `overID`.
- Doc sync: `src/docs/spec/app/06_canvas.md`; `mfb man canvas pollMouse`,
  `mfb man canvas setGroup` and `mfb man canvas types` rendered. `setGroup`'s page gains
  an example of one group drawn twice under two `id`s.
- Final gate: none in this letter; 172-M. `MFB_MOUSE_INJECT` is POSIX-only, so the
  Windows proof of `overID` is in 172-M on `test@192.168.1.206`.

## Open Decisions

172-A's Open Decision 5, answered there; and:

1. **The separator between the parts of an id path.** Recommended: `/`. An `id` commonly
   contains a hyphen (`"save-button"`), so with `-` the path `"save-button-label"` does
   not say where the group's `id` ends; `/` reads as a path and is unlikely inside an
   `id`. The owner's first wording was a `"<groupid>-"` prefix; the alternative is to
   keep `-`. Nothing else in the letter depends on which: it is one character in
   `__canvas_joinId`, the spec and the tests.

   **ANSWER: `::`** (owner, 2026-10-10) — neither of the two above: "it's the
   'namespace' already for the language." A path reads as a qualified name,
   `"toolbar::save::label"`, and a single `:` or a `-` inside an `id` cannot be mistaken
   for it.

## Corrections

- **Owner's decision, 2026-10-10: `overID` is an id path, and `Group` has an `id`.**
  Put to the owner while this letter was pending: should a group and a picture carry an
  `id` that prefixes what they hold? Answer: yes, both — "more than one picture/group can
  use the same shape". §1's goal and non-goals, §3 (the path is built by the walk),
  Compatibility, Phase 2 (the record, the port, the walk, five id-path cases), Phase 3
  (two more injected-event cases, the fast path's condition, the spec) and the
  Validation Plan are rewritten; the non-goal "a group used twice reports the same `id`
  … no path is reported" is withdrawn. `planning/new_canvas.md` is updated to match.
  The picture half lands in 172-L, where a picture gets a shape. The separator is Open
  Decision 1.
- **From 172-A:** P4 and P5 are both CONFIRMED, so §3 stands as written. The P5 probe
  found three things the design did not name — six per-target tables spell
  `canvas.pollMouse`; mouse state is reserved by a `_pollMouse` symbol suffix; and an
  MFBASIC wrapper is compiled into every canvas program whether called or not. Phase 3
  gains two tasks for them.

Found during execution, 2026-10-11:

- **The prerequisite's grep prints 3, not 2.** Both premises have a verdict row; a
  third line of 172-A names both in one sentence.
- **The per-kind cases cannot be unit tests in the binary.** `cargo test --bin mfb
  canvas…` tests read the MFBASIC source as text — nothing there compiles or runs it
  (`helper_render/tests/mod.rs` is all `contains` and `matches`). A point test's
  expectation is a statement about arithmetic, so every per-kind case is a headless
  program in `tests/canvas/rt_canvas_hit.rs`. Phase 1's Check is therefore `cargo test
  --test rt_canvas_hit hit_region`; `--bin mfb canvas_hit` still names two text tests.
- **No debug probe.** The plan drove Phase 2's cases through a probe "the way
  `MFB_CANVAS_STATS` reads are", because it expected them to land before `overID`
  existed. They landed together, and the public path asks the same question with no
  test-only code in the compiler: a press injected with `MFB_MOUSE_INJECT`,
  `canvas::pollMouse`, `overID`. A shape with an `id` reports it exactly when hit, so a
  miss is as visible as a hit. `canvas::enableMouse(TRUE)` re-feeds the injected bytes
  each time it is called, which is what lets one program be asked about several scenes.
- **`__canvas_hitEntry` takes the entry's group translation too**: `(offset, mat, gdx,
  gdy, x, y)`. §3 wrote four parameters; the translation is the entry's
  `__CANVAS_DRAW_DX` / `_DY`, which the matrix number does not carry.
- **The id path is NOT a parameter on `__canvas_appendDraw`; it is a second walk,
  `__canvas_hitAppend`.** Three reasons, each read from the code:
  1. `__canvas_appendDraw`'s leaf builds the RENDERER's record. For a `Text` that
     rasterises every glyph into the calling thread's glyph cache — on the worker, which
     draws nothing, that is the whole text of the scene rasterised a second time.
  2. On the graphics thread the entries `__canvas_appendDraw` makes are re-laid by the
     native `canvas::sceneLayout`, one word per entry per list. It knows no list of
     strings, so a `__CANVAS_DRAW_IDS` appended there would not line up with the
     offsets it returns.
  3. Its leaf folds the draw hash, which a hit list has no use for.
  Both walks call `__canvas_composeGroup` and share the depth stop, so they cannot
  disagree about what a nested group means. The lists are `__CANVAS_HIT_OFFSETS` and
  `__CANVAS_HIT_IDS`, beside the worker's own `__CANVAS_DRAW_DX` / `_DY` / `_MAT`.
- **A `Text` and a `Picture` have their own records on the worker**
  (`__canvas_hitGeometry`): a `Text` is its paint and its layout box with no glyph
  (`__canvas_textBoxHeader`), a `Picture` its rectangle. The box lives in the
  gradient's four point slots, which a `TEXT` record never uses.
  - First cut: the box was written into the RENDERER's text records too
    (`__canvas_withTextBox` in `__canvas_textHeader`), so that 172-K could hit-test text
    on the graphics thread. Measured: `_mfb_ifn_canvas_5FwithTextBox` and its `$own1`
    clone were 6,128 instructions, and the empty canvas program grew 33,024 bytes
    (3,194,380 → 3,227,404, two 16 KB pages). Nothing in this letter reads that box on
    the graphics thread, so it was removed — code for a later letter is not this
    letter's to ship — and the size is back to 3,194,380.
  - **Consequence for 172-K, recorded there:** `__canvas_hitEntry` answers a `Text`
    only over a list `__canvas_hitAppend` built. The renderer's `TEXT` and
    stroked-text `POLYGON` records carry no box.
- **`present` records the id fact with a native pass, not in the signature walk.**
  `__canvas_groupSignature` is MFBASIC and visits only group nodes; an `id` scan there
  would have had to `MATCH` every item of every present, at the ~250 ns a variant
  extraction costs, for every canvas program. `canvas::sceneIdBits(items)`
  (`func_item_hash.rs:lower_scene_id_bits`) reads each item's first field natively —
  `id AS String` on all ten variants, checked against the type model at build time —
  and answers two bits: an item (a group node included) has an `id`; an item is a group
  node. The items INSIDE the groups are not looked at by `present` at all. A scene
  whose bits are 2 (group nodes, none named) is checked by `__canvas_reachesId` when an
  event arrives and before any geometry is built. Measured cost to `present`: none
  visible — 10,000 id-less rectangles present in 864–963 µs after, 871–922 µs at the
  starting commit (`base_hit.sh`, medians of 40, two runs each).
- **There is no `MouseRaw` record; the native poll returns a whole `MouseEvent`.**
  First cut, as §3 wrote it: a six-field raw record and an MFBASIC wrapper that built
  the seven-field event by name. Measured with the bench's id-less program, 59 polls
  timed together: 84 ns a poll, against 33 ns at the starting commit — the wrapper
  allocating and filling a second record on EVERY poll, idle ones included. That is not
  "within noise", so the split moved: the native builds the finished 88-byte block
  (seven slots, `position` at 56, the empty string's sub-block at 72) and the wrapper
  hands it straight back unless the event is over something named. Now 50 ns median, 33
  ns minimum. `a_polled_event_is_a_whole_record` holds the hand-built layout by copying
  polled events into a list and reading every field back.
- **The whole of it is one chunk gated on `pollMouse` being called**, not helpers
  "registered beside the others". An always-compiled wrapper calls the raw poll in
  every canvas program, which reserves a mouse ring in every canvas program (172-A
  finding 4). `HelperGate::WhenUsed` was already the mechanism (`crypto::argon2id`); a
  gated chunk is its own source file and can still see the package's globals and types.
- **`canvas::MouseEvent` changed field-kind class**, which Phase 2's task did not list:
  with a `String` it is a size-varying record. Its `FIELD_KIND_TABLE` entry is
  `SIZE_VARIES`, its `field_kinds.tsv` row takes the size-varying expectations, and
  `field_kinds_gen.py`'s literal for it gains `overID := ""`.
- **The port broke four test targets' COMPILE, and a green runtime suite did not see
  it.** `codemod_i.py` inserted `id := ""` into 120 constructors; 46 of them are inside
  escaped Rust string literals, where a bare `""` ends the string.
  `tests/canvas/rt_canvas_hit.rs`, written after the port, was green throughout. The
  full round found it (`could not compile mfb (test "rt_canvas_rasteriser") due to 52
  previous errors`, and three more). Fixed by `codemod_i2.py`.
- **A dashed open shape is hit as the solid one.** Not decided by the plan or by
  `planning/new_canvas.md`. `__canvas_hitDistance` passes `dashed` FALSE, so a gap
  between two dashes is on the line; the alternative — a line you can only point at
  where its dashes are — makes a dashed line harder to hit than a solid one for no
  reason a program asked for. Stated in `mfb man canvas pollMouse` and the spec.
- **The point tested is the CENTRE of the event's pixel.** `MouseEvent.position` is a
  whole pixel (the ring's coordinates are integers), and the rasteriser evaluates a
  pixel at its centre, so `__canvas_pollMouse` tests `position + 0.5`. Testing the
  corner would put the pointer half a pixel up and to the left of what it is on.
- **An open shape with no stroke is not hit, and a closed one with no paint is.** Both
  follow from what builds a record: `__canvas_segmentHeader`, `__canvas_arcHeader` and
  `__canvas_polylineHeader` decline a paint that does not stroke, and
  `__canvas_rectHeader` declines only a non-positive size. So a rectangle with the zero
  paint is an invisible click target, and a `Line` with the zero paint is nothing.
- **Phase 4's generator polls sixty events a frame, not one.**
  `datetime::monotonicNanos` ticks in microseconds here (every sample was a multiple of
  1000), and one id-less poll is a few tens of nanoseconds. So each frame takes the
  first poll alone and the other fifty-nine together.
- **Phase 4's figures** — `bash tools/canvas-bench/bench.sh target/release/mfb hit`,
  release build, Apple M2 Max, load average 16 at the time (another session's test run),
  medians of 40 frames with the minimum after the slash:

  | Items | ids | first poll after a changed scene | each further poll |
  |---|---|---|---|
  | 1,000 | every item | 760 µs / 756 µs | 8.98 µs / 8.93 µs |
  | 10,000 | every item | 8.63 ms / 8.08 ms | 172 µs / 162 µs |
  | 1,000 | none | under 1 µs | 50 ns / 33 ns |
  | 10,000 | none | under 1 µs | 50 ns / 33 ns |

  The starting commit (`142491ada`, built in a detached worktree) polls the same id-less
  program in 33 ns / 33 ns. So about 0.8 µs an item to rebuild and about 17 ns an item
  to walk. Of the rebuild, reading each item's `id` out of the union (`__canvas_itemId`,
  one variant extraction) is about a quarter at 10,000 items: 6.74 ms with it stubbed
  out against 9.31 ms, the two builds measured back to back (before the native poll
  changed, which is why 9.31 is not the table's 8.63).
  - The same on the boxes, for the two emitters the Mac does not run: box 2226
    (aarch64 Linux) `[i484]` on both named rows, 60 events each; box 2227 (x86-64
    Linux, musl) `[i484]` on both, but only 1 of 60 events at 10,000 named — its first
    poll takes 162 ms there, past the tenth of a second an uncollected event lives, so
    the other 59 were dropped as designed. `bench.sh` prints the count for that reason.
- **Not run: `scripts/coverage-check.sh`.** It reads the profile a whole-suite
  `scripts/coverage.sh` run leaves. What the Validation Plan asked — is `helper_hit.rs`
  in the denominator — is answered from `scripts/coverage-common.sh`: its `IGNORE`
  pattern does not match `src/codegen/builtins/canvas/helper_hit.rs`, and `grep -c
  helper_hit scripts/coverage-exceptions.txt` → 0. The file's line coverage is 172-M's
  final gate to measure.
- **Not done here: the Validation Plan's "`examples/canvas-demo` with one button given
  an `id`".** The demo's rewrite is on hold by the owner's decision; the two-button
  program in `mfb man canvas pollMouse` and `mfb man canvas setGroup` is compiled by
  `cli_canvas_man_examples_compile`, and `rt_canvas_hit` runs the injected click.

## Summary

The hit test is small because the distance functions already exist; it is a walk and a
comparison. The open cost is the worker's second geometry cache, which is why the
letter ends in a measurement rather than a claim, and why a program with no `id`
anywhere never builds one.

As built: 0.8 µs an item to rebuild after a changed scene, 17 ns an item to walk, and
33 to 50 ns a poll when nothing has an `id` — the starting commit's 33.

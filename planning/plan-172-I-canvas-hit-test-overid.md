# plan-172-I: Hit-testing and `MouseEvent.overID`

Last updated: 2026-10-10
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
| plan-172-H complete and archived | `ls planning/plan-172-H-*` → no matches | NOT MET |
| Premises P4 and P5 have verdicts | `grep -cE 'P[45].*(CONFIRMED\|FALSIFIED)' planning/completed/plan-172-A-*.md` → 2 | NOT MET |
| Every shape has an `id` | `grep -c 'id_prop()' src/codegen/builtins/canvas/mod.rs` → ≥ 9 | NOT MET |

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
  its own, joined by the separator (Open Decision 1), with empty parts left out. So:
  - a named shape in the scene itself reports its `id`, as a scene with no named group
    always has;
  - a named shape inside a group with `id` `"save"` reports `"save"`, the separator,
    then its own `id`; nested groups chain;
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
  program that puts the separator inside an `id` gets what it wrote.
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

- [ ] New `src/codegen/builtins/canvas/helper_hit.rs`: `__canvas_hitEntry(offset,
      matrix, x, y) AS Boolean`, built on `__canvas_geoDistance`, the clip path
      distance, and a box test for text. Register it beside the other `helper_*` files.
- [ ] Unit cases beside it, one per kind: a point inside, on the stroke, 2 pixels off a
      1-pixel line (hit), 4 pixels off (miss), inside an unfilled rectangle (hit),
      inside a clipped-away part (miss), inside a rotated ellipse's corner box but
      outside the ellipse (miss).

Acceptance: every per-kind case passes, including the bounding-box-but-not-shape miss.
  Check: `cargo test --bin mfb canvas_hit` → pass (est. 4 min).
Commit: —

### Phase 2 — The draw-list walk

- [ ] `mod.rs`: `Group` gains `id` first (`id_prop()`), and its description says what
      the `id` is for; `helper_render.rs:__canvas_noItem` and the second stand-in node
      in `__canvas_hashLayers` take the field.
- [ ] Port the `canvas::Group[` sites: `id := ""` first. Re-measure the count first
      (118 on 2026-10-10) and record it.
- [ ] `FIELD_KIND_TABLE` and the `canvas::Group` row of `field_kinds.tsv` (its check
      column follows the generator); the IR-embedding goldens.
- [ ] `helper_render.rs:__canvas_appendDraw`: take the id path as a parameter, join a
      group node's `id` onto it for its children, and append each entry's path joined
      with the item's own `id` to `__CANVAS_DRAW_IDS`; reset it wherever the offset
      arrays are reset. One `__canvas_joinId(path, id)` does the joining and leaves an
      empty part out.
- [ ] `helper_hit.rs`: `__canvas_hitAt(x, y) AS Integer`, last entry first.
- [ ] Runtime cases in new `tests/canvas/rt_canvas_hit.rs`, driven through a
      debug-build probe function the way `MFB_CANVAS_STATS` reads are: two overlapping
      shapes report the upper; an item inside a rotated, scaled group is hit at its
      transformed place and missed at its untransformed one; an item in a layer above
      wins over one in a layer below.
- [ ] Id-path cases in the same file, read through the same probe: one group drawn
      through two nodes with different `id`s reports two different paths for the same
      item; an unnamed item inside a named group reports the group's `id` alone; a named
      item inside an unnamed group reports its own `id` alone; a group inside a group
      reports both, outermost first; nothing named on the path reports `""`.

Acceptance: the three hit cases and the five id-path cases pass, and every group test
passes with no PNG regenerated.
  Check: `cargo test --test rt_canvas_hit --test rt_canvas_group_ownership --test
  rt_canvas_golden groups` → pass (est. 8 min); `git status --porcelain
  tests/golden/canvas` → empty.
Commit: —

### Phase 3 — `overID`

- [ ] `mod.rs`: append `overID AS String` to `MouseEvent`; add the internal raw record.
- [ ] `func_poll_mouse.rs`: the native becomes `pollMouseRaw`, `internal_only`; add the
      MFBASIC `pollMouse` body and the lazy worker draw list with its three conditions.
- [ ] Rename `canvas.pollMouse` to `canvas.pollMouseRaw` in the six per-target tables
      (`src/target/{macos_aarch64,win_x86_64,linux_common}/{mod,plan}.rs`) and move the
      `src/codegen/registry/mod.rs` record-returning entry (172-A finding 2).
- [ ] Make the three mouse-state suffix tests (`src/target/shared/plan/symbols.rs:
      module_uses_mouse`, two in `src/codegen/engine/builder/mod.rs`) accept the raw
      poll's symbol, and register the MFBASIC `pollMouse` so a canvas program that never
      calls it does not emit the raw poll or reserve mouse state (172-A findings 3, 4).
      Check: an empty canvas program's executable is the same size before and after
      this phase.
- [ ] `func_present.rs` / `func_present_layers.rs`: record whether the published scene
      has any non-empty `id` — an item's or a group node's, the items of the groups it
      names included (the walk `__canvas_groupSignature` already makes).
- [ ] Runtime cases in `tests/canvas/rt_canvas_hit.rs` under `MFB_MOUSE_INJECT`: a
      press over a shape with an `id` reports it; over empty canvas reports `""`; over
      an id-less shape lying on an id-bearing one reports `""`; after a `present` that
      moves the shape, the same position reports the new answer; a press over each of
      two uses of one group reports that use's `id` in front of the item's; and a scene
      whose only `id` is on a group node still resolves `overID` (the fast path must not
      skip it).
- [ ] Regenerate `tests/syntax/app/app-mouse-surface` goldens per
      `.ai/testing-gates.md` and port its source to read `overID`.
- [ ] Port the 23 `pollMouse` mentions where they construct or destructure the record.
- [ ] `src/docs/spec/app/06_canvas.md`: a "Hit-testing" section stating the region
      rules, the id path (its order, the separator, the empty-part rule) and when
      `overID` is resolved, cited to `helper_hit.rs` and `__canvas_appendDraw`; "Named
      groups" gains the node's `id`;
      `src/docs/spec/language/18_builtin-functions.md` where it lists `MouseEvent`.

Acceptance: the four injected-event cases pass.
  Check: `cargo test --test rt_canvas_hit over_id` → pass (est. 4 min).
Commit: —

### Phase 4 — Cost, measured

- [ ] Add a `hit` generator to `tools/canvas-bench/gen/` on the pattern of `worker.sh`:
      N items with ids, the scene changed and one event polled per frame.
- [ ] Record, on a release build, the per-poll time at 1,000 and 10,000 items, and the
      same with no ids (the fast path), in Corrections.
- [ ] State the two figures in `canvas::pollMouse`'s registry description.
- [ ] Update `tools/canvas-bench/README.md` for the new generator.

Acceptance: four measured figures are recorded, and the no-id figure is within noise of
the figure taken at this letter's starting commit.
  Check: `bash tools/canvas-bench/bench.sh target/release/mfb hit` → four figures
  (est. UNMEASURED — time the first run and record it).
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

   **Unanswered. Phase 2's id-path cases and Phase 3 cannot start until it is.**

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

## Summary

The hit test is small because the distance functions already exist; it is a walk and a
comparison. The open cost is the worker's second geometry cache, which is why the
letter ends in a measurement rather than a claim, and why a program with no `id`
anywhere never builds one.

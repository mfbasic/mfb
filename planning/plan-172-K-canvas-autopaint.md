# plan-172-K: The `Shape` / `AutoShape` / `DrawItem` unions and `AutoPaint`

Last updated: 2026-10-10
Effort: large (3h–1d)
Depends on: plan-172-J

The item types become three nested unions, and `canvas::AutoPaint` wraps a shape with
up to four alternative paints that the graphics thread selects from the pointer: enter,
hover, pressed and exit.

Behavioral outcome: a program presents a scene holding an `AutoPaint` once, never
presents again, and the shape changes paint as the pointer enters it, rests on it,
presses it and leaves it — including the 50 ms enter and exit paints expiring with the
pointer still — while a scene with no `AutoPaint` still costs zero frames when the
pointer moves.

References:

- `planning/new_canvas.md` — "Add new", "AutoPaint states", "Where AutoPaint is
  resolved: the graphics thread", "Groups and identity", "Hit-testing".
- `planning/plan-172-A-canvas-shapes-premises.md` §4 P1 and P2.
- `.ai/canvas-threading.md` §4 (the five redraw triggers), §8 (the race matrix), §14
  (item hashes), §15 (`waitFrame` and the two condition variables).
- `src/codegen/builtins/canvas/helper_render.rs` — `__canvas_appendDraw`,
  `__canvas_renderLoop`, `__canvas_renderFrame`.
- `src/codegen/runtime/canvas/mod.rs` — `emit_wait_for_redraw`, `emit_signal_redraw`,
  `emit_await_pickup` (the timed-wait precedent).
- `src/codegen/builtins/canvas/helper_hit.rs` (172-I), `func_pointer.rs` (172-J).

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-J complete and archived | `ls planning/plan-172-J-*` → no matches | NOT MET |
| Premises P1 and P2 have verdicts | `grep -cE 'P[12].*(CONFIRMED\|FALSIFIED)' planning/completed/plan-172-A-*.md` → 2 | NOT MET |
| If P2 was FALSIFIED, its bug is fixed | `grep -c 'P2.*CONFIRMED' planning/completed/plan-172-A-*.md` → 1, or the bug-NN it names is archived | NOT MET |
| The scripted pointer works | `cargo test --test rt_canvas_pointer` → pass | NOT MET |

If plan-172-J is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `canvas::Shape` is the eight shapes; `canvas::AutoShape` is `Shape` plus `AutoPaint`;
  `canvas::DrawItem` is `AutoShape` plus `Picture` and `Group`.
- `canvas::AutoPaint` is exactly the nine-field record in `planning/new_canvas.md`.
- The state rules in "AutoPaint states" hold, each proved by a test:
  priority `pressed`, `enter`, `hover`, `exit`, default; an unused state falls through;
  `enter` and `exit` last 50 ms; re-entry and early exit cancel; `pressed` needs the
  press to have begun inside; only the topmost item under the pointer is affected.
- State is resolved on the graphics thread and survives a re-`present` of the scene.
- Two redraw triggers are added: a pointer change while the live scene holds an
  `AutoPaint`, and an enter or exit deadline expiring.
- The hit region is built from the shape's own `paint`, never from a state paint.

### Non-goals (explicit constraints)

- **`Picture` is not reshaped here.** It joins `DrawItem` as it is; 172-L gives it a
  `shape`.
- **No interpolation between paints.**
- **No callback, event or `canvas::` query reports an `AutoPaint`'s state to the
  program.** `overID` (172-I) is the program's view of the pointer.
- **The worker never resolves state** and `present` never blocks on it.
- **The 50 ms duration is a constant**, not a field.
- **A scene with no `AutoPaint` renders no extra frame** for any pointer activity.

## 2. Current State

`DrawItem` is one flat registry union; `RegistryUnion` has no way to express `INCLUDES`.
After 172-E it lists ten records.

The render loop is `WHILE canvas::waitForRedraw()`, an untimed condition wait on the
graphics mutex; `emit_signal_redraw` sets the pending flag and signals. There is no
timed wake. `canvas::waitFrame`'s native half, `emit_await_pickup`, is the one timed
condition wait in the package, on a different condition variable.

The walk `__canvas_appendDraw` turns each item into a geometry offset and a draw hash.
A picture already folds extra state — its pixel-block address — into the draw hash so
that a change the worker's published hash cannot see still repaints.

After 172-J the graphics thread can read the pointer; after 172-I it can ask which draw
entry is topmost under a point.

### Measured populations

| What | Count | Command |
|---|---|---|
| `DrawItem` variants entering this letter | 10 | `cargo test --bin mfb draw_item_variant_set_is_frozen` passes against the ten-name list |
| `CASE` arms over item variants in the companion source | 55 today, `RoundedRect` included | `grep -rhoE 'CASE (Rectangle\|RoundedRect\|Line\|Polygon\|Circle\|Arc\|Text\|Ellipse\|PolyLine\|Picture\|Group)\(' src/codegen/builtins/canvas \| wc -l` → 55; re-measure on entry. Each enclosing `MATCH` needs an `AutoPaint` arm or a `CASE ELSE` that is correct for it |
| Native variant lists | 2 | `grep -n 'const HASHED\|const NOT_HASHED' src/codegen/builtins/canvas/func_item_hash.rs` → `HASHED: [&str; 9]`, `NOT_HASHED: [&str; 2]` |
| Redraw triggers today | 5 | `.ai/canvas-threading.md` §4's table |
| Frozen-set tests that name the variant list | 3 | `draw_item_variant_set_is_frozen`, `every_draw_item_variant_has_a_record`, `every_draw_item_variant_carries_a_paint` in `mod.rs` |

### Verified properties

- **A draw hash can carry state the worker did not publish.** VERIFIED by reading
  `__canvas_appendDraw`'s picture branch (bug-484).
- **A graphics-thread `FAIL` has nowhere to unwind to.** VERIFIED from the comment in
  `__canvas_appendDraw`'s group arm. Nothing in the state machine may raise.
- **`AutoPaint` cannot nest.** VERIFIED from the type: its `shape` is a `Shape`, and
  `Shape` does not contain `AutoPaint`.
- **UNVERIFIED — that a timed wait in `emit_wait_for_redraw` leaves `MFB_CANVAS_SYNC`
  and `waitFrame` correct.** Phase 3 runs their suites.

## 3. Design Overview

**The unions.** Declared with `INCLUDES` if P1 was CONFIRMED, otherwise as three flat
unions with the same members. Member order is `planning/new_canvas.md`'s. The tags
renumber once more; nothing may depend on the old order, and
`draw_item_variant_set_is_frozen` is amended to the eleven names
(`Rectangle, Line, Polygon, Circle, Arc, Text, Ellipse, PolyLine, AutoPaint, Picture,
Group`), with two sibling tests pinning `Shape` and `AutoShape`.

**Expansion.** `__canvas_appendDraw` gains `CASE AutoPaint(a)`. It appends two things
per instance: the entry for the wrapped shape with its own paint — which is what the
hit test reads — and a record in a per-frame list of `AutoPaint` instances (entry index,
instance key, the four flags and paints).

**Instance key.** A hash of the instance's structural path — the chain of group slots
and item indices that reached it, from the scene down. No `id` is consulted (Open
Decision 2): an `id` is what `overID` reports to the program, and it does not have to be
unique, so it cannot be what tells two instances apart. Every use of a group is its own
chain, so every use has its own state whatever it is called.

What a structural key costs: an instance's state follows its place in the scene, so
inserting or removing an item ahead of it in the same list gives it a new key, and it
starts again from default. That is one `enter` transition on a frame where the scene was
restructured under the pointer. Moving, restyling or re-presenting an item in place
keeps its key.

**Resolving a frame.** After the walk, once, on the graphics thread:

1. Read the pointer snapshot and the monotonic clock.
2. If the pointer is inside the surface, `__canvas_hitAt` gives the topmost entry.
3. For each instance, advance its state from the table kept across frames: inside or
   not, button edge, deadlines.
4. For each instance whose state is not default, replace its entry's geometry offset and
   draw hash with those of the shape under the chosen paint.
5. Drop table rows for keys not seen this frame.
6. Publish the earliest pending deadline, or none, and whether any instance exists.

Step 4 is what makes damage and the geometry cache correct: a changed paint is a changed
hash.

**The table** is graphics-thread globals: parallel lists of key, state, deadline and
the key that owns the current press.

**Trigger 6 — pointer change.** The graphics thread stores "the live scene holds an
`AutoPaint`" in a graphics-state word at step 6. Each platform's pointer publication
(172-J) reads it and calls `canvas::signalRedraw` only when it is set.

**Trigger 7 — deadline.** The graphics thread stores the earliest deadline in a
graphics-state word. `emit_wait_for_redraw` uses `pthread_cond_timedwait` when it is
non-zero and reports a timeout as a redraw. The clock mismatch is handled in one place:
deadlines are kept in the monotonic clock the state machine reads and converted to the
realtime clock the timed wait takes at the moment of waiting.

**A pointer-only frame that changes nothing is not rendered.** If the frame was woken by
trigger 6 or 7 alone and no instance's chosen paint changed, the frame ends after step
6 — the same outcome as an empty damage union.

**Where the risk is:** this is the first time the graphics thread changes what is drawn
without the worker publishing anything, and the first time its wait can time out. The
race rows this adds to `.ai/canvas-threading.md` §8 are listed in Phase 4, and each has
a test.

**Gate class: new behaviour.** Existing goldens must hold: with no `AutoPaint` in a
scene, steps 2–5 do nothing and both state words are zero.

### Rejected alternatives

- **Resolving on the worker.** Rejected by the owner: hover would stop while the
  program is blocked.
- **A timer thread for the deadline.** Rejected: the graphics thread is already the
  thread that sleeps until there is something to draw; a second sleeper would need the
  same lock.
- **Rebuilding state from scratch each frame.** Rejected: a program that presents every
  frame would restart its 50 ms window every frame and never reach `hover`.
- **Keying state on the item hash.** Rejected: the hash changes whenever the shape
  moves, which would reset the state of anything animated.

## Compatibility / Format Impact

- **Added:** `canvas::Shape`, `canvas::AutoShape`, `canvas::AutoPaint`; two
  graphics-state words; two redraw triggers.
- **Changed:** `DrawItem`'s member order and tags; `emit_wait_for_redraw` may time out.
- **Unchanged:** the scene ring, `present`, the group table, `ITEM_BLOCK_SIZE`, both
  shaders, every pixel of a scene with no `AutoPaint`.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — The unions and the record, drawing its own paint

Lands the types with a complete, if modest, behaviour: an `AutoPaint` draws its shape.

- [ ] Re-measure the `CASE` arm count and list the enclosing `MATCH`es by function.
- [ ] `src/codegen/registry/mod.rs`: `RegistryUnion.includes` and its rendering, if P1
      was CONFIRMED.
- [ ] `mod.rs`: declare `Shape`, `AutoShape`, `DrawItem` and the `AutoPaint` record with
      field descriptions that state each rule from "AutoPaint states".
- [ ] Amend `draw_item_variant_set_is_frozen` to the eleven names, with the four answers
      in the commit; add `shape_variant_set_is_frozen` and
      `auto_shape_variant_set_is_frozen`; extend
      `every_draw_item_variant_carries_a_paint`'s container list with `AutoPaint` only
      if it declares no `paint` — it does not, its shape carries one.
- [ ] `__canvas_appendDraw`: `CASE AutoPaint(a)` appends the wrapped shape's entry.
      Every other `MATCH` over `DrawItem` in the companion source gets its arm.
- [ ] Native: add `AutoPaint` to the variant handling in `func_item_hash.rs`,
      `func_geo_build.rs` and `func_carried_hashes.rs` so it is hashed through its
      fields and built through the MFBASIC path.
- [ ] `__canvas_groupSignature`, `__canvas_closeRetired` and the
      `__canvas_listNames*` helpers: an `AutoPaint` wrapping a `Text` names a font.
- [ ] Runtime case in new `tests/canvas/rt_canvas_autopaint.rs`: an `AutoPaint` with no
      pointer draws exactly what its bare shape draws, in a scene and inside a group,
      and survives 200 present / drop cycles with no leak.

Acceptance: an `AutoPaint` renders as its shape, with every existing golden unchanged.
  Check: `cargo test --test rt_canvas_autopaint --test rt_canvas_golden --test
  rt_canvas_present_leak --test rt_canvas_group_ownership` → pass (est. 12 min).
Commit: —

### Phase 2 — The state machine, as a pure function

- [ ] New `helper_autopaint.rs`: `__canvas_autoStep(state, deadline, pressOwner, inside,
      buttons, wasButtons, flags, now)` returning the next state and deadline. No
      globals, no drawing.
- [ ] Unit cases beside it, one per rule in §1: priority order; each unused state's
      fall-through; enter expiring to hover at 50 ms; exit expiring to default;
      re-entry during exit; leaving during enter; a press begun inside, dragged out and
      back; a press begun outside and dragged in.

Acceptance: every rule in "AutoPaint states" has a passing case named for it.
  Check: `cargo test --bin mfb canvas_auto_step` → pass (est. 4 min).
Commit: —

### Phase 3 — Resolution on the graphics thread, and the two triggers

- [ ] `helper_autopaint.rs`: the instance list, the table, and the six-step resolve
      called from `__canvas_renderFrame` after the walk.
- [ ] `src/codegen/runtime/canvas/mod.rs`: the `AUTOPAINT_LIVE` and `REDRAW_DEADLINE`
      words; `emit_wait_for_redraw` takes the timed branch and returns a timeout as a
      redraw; `emit_publish_pointer` signals when `AUTOPAINT_LIVE` is set.
- [ ] `helper_render.rs`: the debug `MFB_CANVAS_POINTER` script's next entry time joins
      the deadline, so a scripted test wakes without a present.
- [ ] Runtime cases in `tests/canvas/rt_canvas_autopaint.rs`, each one `present` and a
      pointer script, read through `MFB_CANVAS_DUMP`:
      hover paint shows after entry; the enter paint shows first and gives way with the
      pointer still; exit shows then default; pressed wins over hover; only the upper of
      two overlapping `AutoPaint`s changes; an id-less shape on top blocks the one
      beneath; two uses of one group change independently — under two group `id`s,
      under the same `id`, and under none, each holding a NAMED `AutoPaint`; a hover
      paint with a wider
      stroke does not widen the hit region; state survives a re-`present` of a moved
      shape.
- [ ] A frame-count case: a scene with no `AutoPaint` and a moving scripted pointer
      renders exactly one frame; with one `AutoPaint` and the pointer never over it, the
      pointer-only wakes render none after the first.
- [ ] Run `rt_canvas_frame_stats` and `rt_canvas_graphics_thread` unchanged.

Acceptance: all nine state cases and the frame-count case pass, and the `waitFrame` and
graphics-thread suites still pass.
  Check: `cargo test --test rt_canvas_autopaint --test rt_canvas_frame_stats --test
  rt_canvas_graphics_thread` → pass (est. 10 min).
Commit: —

### Phase 4 — GPU parity, the race rows and the documents

- [ ] A Metal parity case for a hovered scene in `tests/canvas/rt_canvas_metal.rs`, and
      the scene added to `scripts/test-canvas-gpu-rows.sh`; run the Vulkan row on box
      2226.
- [ ] Race rows, each a test in `tests/canvas/rt_canvas_graphics_thread.rs` using
      `MFB_CANVAS_FRAME_HOLD_MS`: the pointer changes while a frame is in flight (the
      frame completes with the state it resolved; the next has the new one); a `present`
      lands during an enter window (the window is not restarted); the graphics thread
      stops while a deadline is pending (shutdown completes); `waitFrame` is in progress
      when a deadline expires (it returns as it would have).
- [ ] `.ai/canvas-threading.md`: §4's table grows to seven triggers and its "time is not
      among them" sentence is rewritten to name the one bounded exception; §8 gains the
      four rows; §9's "No time-based repaint" entry is corrected; a new section
      describes the instance table and why it is keyed as it is.
- [ ] `src/docs/spec/app/06_canvas.md`: an "AutoPaint" section with the state rules, the
      two triggers and the hit-region rule, cited to `helper_autopaint.rs`.
- [ ] Man examples for `AutoPaint`, gated by
      `tests/cli/cli_canvas_man_examples_compile.rs`.

Acceptance: the four race rows pass deterministically, and Metal and Vulkan match the
software frame of a hovered scene within `Tolerance::GPU_DEFAULT`.
  Check: `cargo test --test rt_canvas_graphics_thread auto --test rt_canvas_metal auto`
  → pass, and `scripts/test-canvas-vulkan.sh target/release/mfb --box 2226` → pass
  (est. 15 min).
Commit: —

## Validation Plan

- Tests: `tests/canvas/rt_canvas_autopaint.rs` (new); the state unit cases; four race
  rows; one Metal case and one Vulkan row; three frozen-set tests.
- Coverage check: `scripts/coverage-check.sh` shows `helper_autopaint.rs` in the
  denominator.
- Runtime proof: `examples/canvas-demo`'s menu buttons wrapped in `AutoPaint`, run in a
  real window on macOS with the program blocked in a key read; the button under the
  pointer changes paint.
- Doc sync: `.ai/canvas-threading.md`; `src/docs/spec/app/06_canvas.md`;
  `mfb man canvas types` rendered and read for `AutoPaint`'s nine fields.
- Final gate: none in this letter; 172-M.

## Open Decisions

1. **Which buttons count as pressed.** `planning/new_canvas.md` says any button.
   Recommended: keep that. The alternative is the left button only, which is what most
   toolkits mean by "pressed".
2. **Two `AutoPaint` instances with the same id path in one frame.** The design says
   each use of a group gets its own state, and the instance key is the id path when
   there is one — so a named `AutoPaint` inside a group drawn twice under the same
   group `id` (or none) would share one key, and hovering one would light both. This
   was already true of the key as first written (the item's own `id`); 172-I's group
   `id` gives the program the way to tell the uses apart, and this decides what happens
   when it does not. Recommended: when an id path occurs more than once in a frame,
   every instance with that path is keyed by its structural path instead, so the uses
   stay independent and only lose their state across a reordering. The alternative is
   to let them share, and document that independent uses need distinct group `id`s.

   **ANSWER: independent** (owner, 2026-10-10) — "ID is more for mouse overId, not for
   AutoPaint logic." Taken at its word, that removes the `id` from the key altogether
   rather than adding a fallback for when it repeats: the key is the structural path,
   always (§3 "Instance key"). That last step is this plan's reading of the answer, not
   the owner's words.

## Corrections

- **`AutoPaint` has no `id` (owner's decision, 2026-10-10).** "Auto paint is a decorated
  version of a shape, it's not 'using' a shape like Picture or containing a shape like
  groups." It adds nothing to the id path: an `AutoPaint` reports its shape's `id`, as
  the bare shape would, and the record stays as `planning/new_canvas.md` lists it.
- **The instance key is structural (owner's answer to Open Decision 2, 2026-10-10).**
  `id` plays no part in `AutoPaint`'s state. §3's "Instance key" is rewritten, with what
  a structural key costs, and the "two uses of one group" test case covers named
  instances under equal and empty group `id`s.
- **From 172-I as built (2026-10-11): §3's step 2 does not exist yet in the form it
  assumes.** "`__canvas_hitAt` gives the topmost entry" needs four things 172-I did not
  leave on the graphics thread, each read from `helper_hit.rs`:
  1. **The hit test is compiled only into a program that calls `canvas::pollMouse`**
     (`HelperGate::WhenUsed(&["pollMouse"])`): an always-compiled wrapper would have
     made every canvas program reserve a mouse ring. A program with an `AutoPaint` and
     no `pollMouse` has no `__canvas_hitAt`. Phase 3 must either widen the gate to the
     use of `AutoPaint`, or move the region test (`__canvas_hitEntry` and what it
     calls) into the always-compiled source and leave `pollMouse`'s half gated. Measure
     the second: it is code in every canvas program.
  2. **`__canvas_hitAt` walks `__CANVAS_HIT_OFFSETS`**, the list `__canvas_hitAppend`
     builds on the worker, beside the calling thread's `__CANVAS_DRAW_DX` / `_DY` /
     `_MAT`. On the graphics thread the frame's list is the `offsets`
     `__canvas_sceneOffsets` returns; step 2 has to point the walk at it.
  3. **The renderer's text records carry no layout box.** `__canvas_hitEntry` answers a
     `Text` only over a list `__canvas_hitAppend` built, where a `Text` is
     `__canvas_textBoxHeader`'s record. On the graphics thread a `TEXT` record is a
     glyph run and a stroked `Text` is a `POLYGON`, so a label lying on an `AutoPaint`
     button would not be found, and "only the topmost item under the pointer is
     affected" would be wrong for the commonest button there is. 172-I's first cut
     wrote the box into every text record (`__canvas_withTextBox`, the gradient's four
     point slots, `__CANVAS_GEO_GRADIENT_KIND` = 2 as the mark): 6,128 instructions and
     33,024 bytes on every canvas program, measured, and removed because nothing in
     172-I read it. That is the known-working design and its known cost; Phase 3
     decides whether to pay it or to gate it with the rest.
  4. **`canvas::sceneIdBits` fails the BUILD on a `DrawItem` variant whose first field
     is not `id AS String`** — deliberately, so a new variant cannot be read from the
     wrong slot. `AutoPaint` has no `id` (owner's decision above), so Phase 1 has to
     teach `lower_scene_id_bits` what an `AutoPaint` is — its wrapped shape's `id` —
     and give `__canvas_itemId` and `__canvas_hitAppend` an `AutoPaint` arm that
     reports and hit-tests the shape.
- **From 172-I's id path (owner's decision, 2026-10-10):** Open Decision 2 was added for
  two instances whose id paths are equal. The original wording — "the item's `id` when it is non-empty … two
  uses of one group therefore get two keys" — did not hold for a named item, whose `id`
  is the same in every use.

## Summary

The state machine is a pure function with a case per rule, landed before anything calls
it. The risk is in the two places the threading model bends — a wait that can time out,
and a frame the worker did not ask for — and each bend gets a race row in
`.ai/canvas-threading.md` and a deterministic test.

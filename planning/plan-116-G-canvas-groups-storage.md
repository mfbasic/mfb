# plan-116-G: Named groups — storage, lifetime, resolution, and software rendering

Last updated: 2026-08-31
Effort: large (3h–1d)
Depends on: plan-116-F

A scene is a flat `List OF DrawItem` that `present` deep-copies in full every frame
(`func_present.rs` DESC: *"`present` **copies the whole scene**"*). A program with a
large static sub-picture — a map, a sprite sheet, a UI panel — pays that copy on every
`present` even though the sub-picture never changes.

This letter adds **named groups**: a sub-scene installed once under a name, referenced
from a scene by a lightweight node that carries only a name and a translation.

```
canvas::setGroup(name AS String, items AS List OF DrawItem) AS Nothing
canvas::removeGroup(name AS String) AS Nothing

canvas::Group
    dx      AS Float   ' transform the location of the group.
    dy      AS Float   ' transform the location of the group.
    name    AS String  ' the group name, if not found this is a no-op.
```

`canvas::Group` becomes the tenth `DrawItem` variant.

The feature's two goals, recorded from the user (2026-09-01) so later trade-offs
are made against them: **(1) reuse** — a sub-picture authored once and referenced
from many scenes and positions; **(2) speed, CPU and GPU** — `present` copies one
node instead of the sub-picture, the geometry cache holds one entry per shared
group, and plan-116-H draws a group as one instanced draw per node.

Behavioral outcome: a program calls `setGroup("panel", […])` once, then presents
`[Group[dx := 10.0, dy := 20.0, name := "panel"]]`, and the panel renders translated by
(10, 20). Presenting the same scene again copies only the one node, not the panel.
Calling `setGroup("panel", …)` again with different contents changes what is drawn
without any `present`-visible change to the scene list. `removeGroup("panel")` makes
the node a silent no-op, and frees the group's buffer once no scene and no parent group
still reference it and no in-flight frame is still drawing it.

References:

- `.ai/canvas-threading.md` — §2 (arena state is per-thread), §3 (the scene ring and
  its retirement rule), §7 (deferred free and why there is no refcount for textures).
  **Read all three before touching this letter.**
- `src/codegen/builtins/canvas/func_present.rs` — `__canvas_present` and the
  publish-then-render split.
- `src/codegen/builtins/canvas/gen_present.rs:emit_publish` — the deep copy.
- `src/codegen/builtins/canvas/mod.rs:1161` — `every_draw_item_variant_carries_a_paint`,
  which `Group` does not satisfy. §2.
- `src/rules/table.rs:1015` — `TYPE_RESOURCE_FIELD_FORBIDDEN`, retired by plan-114
  but still relevant: `canvas` has not yet migrated off its value handles
  (plan-116-I), which is why resource ownership is **not** in this letter.

## Prerequisites

See plan-116-A §Prerequisites for the three environment gates.

| Must be true | Command | Status |
|---|---|---|
| plan-116-F complete and archived | `ls planning/completed/plan-116-F-*` → one match | NOT MET |

If plan-116-F is not complete, this letter cannot start, full stop.

**Neither plan-114 nor plan-116-I is a prerequisite of this letter** — see §2's "no
resource can reach a group today" finding. Group *resource ownership* is
plan-116-J, which is gated on plan-116-I.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop.

## 1. Goal

- `canvas::setGroup` installs a named sub-scene; `canvas::removeGroup` drops the name.
- `canvas::Group` is the tenth `DrawItem` variant and renders its named group
  translated by `(dx, dy)`, on the software path.
- A nested `Group` inside a group's item list **stays a reference** — it is not
  flattened into the parent's buffer.
- Nesting works to depth **64**; exceeding it raises from `present`.
- A `Group` naming an absent group is a **silent no-op**.
- A group's buffer is freed once nothing references it and no in-flight frame draws it.
- `present` resolves every name to a buffer handle before publishing, so the graphics
  thread never does a string lookup.

### Non-goals (explicit constraints)

- **No resource ownership.** `setGroup` "taking ownership of resources in the list"
  is plan-116-J, gated on plan-116-I (which puts a `RES canvas::Image`/`RES
  canvas::Font` directly in `Picture`/`Text`). Until I lands, no `DrawItem` carries
  a resource (§2), so there is nothing to own — not a deferral of live behaviour, a
  statement that it is currently vacuous.
- **No GPU group draw.** The instanced per-group draw and the two-stage offset are
  plan-116-H. This letter renders groups on the software path and both `*Renderable`
  predicates **decline any scene containing a `Group`**, so the GPU never draws one
  wrongly.
- **`Group` carries no `paint`.** It is a container, not a drawable. §2 handles the
  test this contradicts.
- **`Group` carries a translation only**, not a full `Transform`. That is what was
  specified, and `Paint.transform` on the group's own items already covers the rest.
- **No `Group` inside a `DrawLayer`'s special handling.** A layer's items are
  `DrawItem`s, so a `Group` works there for free; nothing layer-specific is added.
- **No existing golden may move.**

## 2. Current State

### The two invariants this letter contradicts, and how each is handled

**(a) `every_draw_item_variant_carries_a_paint` (`mod.rs:1161`).** The test asserts
every union variant has a `paint AS Paint` field, with the rationale *"a variant that
forgot it would silently draw with no way to colour it."*

`Group` has no paint by design — it draws nothing itself; its children carry their own.
Applying `AGENTS.md`'s four-question gate:

1. **When/why written** — plan-98-A, to make `Paint` a threaded value rather than
   ambient state.
2. **Behaviour it protects** — every *drawable* variant is colourable.
3. **Who else depends** — nothing reads the test's guarantee; `__canvas_headerFor`
   `MATCH`es variants individually.
4. **Proof it is wrong** — none, and none is claimed. The test is right about its
   subject. What changes is that the union gains its first **container** variant, which
   the test's premise ("a variant that forgot it would silently draw") does not
   describe: a `Group` with a `paint` would be a field no renderer could honour.

So the test is **narrowed to its actual subject, not weakened**: it asserts every
variant *other than the container set* carries a paint, with the container set spelled
as an explicit list (`["Group"]`) so a future variant is not silently exempted. The doc
comment records plan-116-G as the reason.

**(b) `draw_item_variant_set_is_frozen` (`mod.rs:1109`).** Already amended once by
plan-116-E for `Ellipse`; this letter appends `"Group"` **last**, by the same
mechanism and for the same recorded reason. Order is append-only because it fixes the
union tags.

### No resource can reach a group today

`TYPE_RESOURCE_FIELD_FORBIDDEN` (`2-203-0084`) is **retired** — plan-114 A–E all
landed 2026-09-01 (`ls planning/completed/plan-114-*` → 5 matches; the rule sits
reserved-not-emitted at `src/rules/table.rs:1015` under a "retired by plan-114-B"
comment). A record field CAN now hold a `RES`. What has NOT changed is `canvas`
itself: `Picture` still names its image through the `ImageRef` value handle and
`Text` through `FontRef` (the `ImageRef`/`FontRef` records in `mod.rs`) — the
migration to direct `RES`
fields is plan-116-I.

So every `DrawItem` variant's fields are value types — `Picture` names an image through
an `ImageRef` (a plain `Integer` id, `mod.rs:397`) and `Text` through a `FontRef` — and
`mod.rs:385` states the consequence outright: *"This is what keeps the scene from
retaining anything."*

**`setGroup` therefore takes ownership of nothing, because nothing ownable can be
in the list.** That is a measured fact about today's `canvas` package, not a
simplification. plan-116-I puts the resources in; plan-116-J makes the ownership
real.

### The frame skip, and the problem groups create for it

`__canvas_present` (`func_present.rs:BODY`) renders only when `canvas::publishScene`
reports a change, and `publishScene` compares the incoming scene's *content* against
the published one (`.ai/canvas-threading.md` §3.1).

**A group breaks that.** `setGroup("panel", …)` changes what is drawn without changing
any scene item — the `Group` node still holds the same `dx`, `dy` and `name`. A program
that calls `setGroup` and then `present` with an unchanged scene list would publish
nothing and render nothing. This is not in the feature request and it is the single
most likely way the feature ships broken. §4.4 fixes it with a per-group revision
counter folded into the comparison.

### Measured populations

| What | Count | Command |
|---|---|---|
| `DrawItem` variants after plan-116-E | 9 | `mod.rs` union list |
| Tests pinning/iterating the variant list | 3 | `mod.rs:1109`, `:1139`, `:1161` |
| plan-114 letters completed | 5 | `ls planning/completed/plan-114-*` (2026-09-01) |
| Ban on resource record fields | retired by plan-114 | `sed -n 1008,1019p src/rules/table.rs` — reserved, never emitted |
| Process-global canvas state symbols today | ~~1~~ **3** | `sed -n 906,932p src/codegen/engine/builder/mod.rs` — `graphics_state_data_object()`, `CANVAS_SCENE_SYMBOL`, `CANVAS_FONTS_SYMBOL` (**G4**) |
| `mfb man canvas` members with compile-gated examples | 13 | `sed -n 23,37p tests/cli_canvas_man_examples_compile.rs` |

> **Census re-verified 2026-09-02 (pre-execution).** All six rows still hold exactly
> after plan-116-C, D and E: 9 `DrawItem` variants, 3 tests pinning or iterating the
> variant list, 5 archived plan-114 letters, the resource-field ban still
> reserved-not-emitted, 1 process-global canvas symbol, 13 compile-gated man members.
> Unlike plan-116-I's (see its **I1**), none of these count *sites this letter must
> sweep* — they are structural facts — which is why they did not move while I's did.

> **Census re-verified again 2026-09-03, after plan-116-F landed.** All six counts are
> unchanged — 9 variants (`Picture, Rectangle, Line, Polygon, Circle, Arc, Text,
> RoundedRect, Ellipse`), 3 pinning tests, 5 archived plan-114 letters,
> `2-203-0084` still reserved-and-never-emitted (its only in-tree uses are the two
> `ir/verify/tests.rs` assertions that it is *not* raised, plus the spec's history
> note), 1 process-global canvas symbol (`CANVAS_SCENE_SYMBOL`), 13 compile-gated man
> members — except the process-global count, which was wrong when written (**G4**).
>
> **The line numbers this letter cites have not held.** F added the `GradientKind`
> enum and the `GradientStop`/`Gradient` records to `mod.rs`, moving the three pinning
> tests down by ~226 lines. See **G1**.

### Verified properties

- **Arena state is per-thread**, so a group table in arena globals would be invisible
  to the graphics thread — the same fact that forced the scene ring to be a
  process-global data symbol. Read `.ai/canvas-threading.md` §2, which states the
  consequence explicitly and rejects the `MAIN_ARENA_GLOBAL_SYMBOL` escape hatch.
  **The group table must therefore be process-global storage, like the scene ring.**
- **Only the worker frees.** `.ai/canvas-threading.md` §3 "Who frees": *"Only the
  worker, and only blocks the worker allocated. An arena is per-thread, so a
  cross-thread free would corrupt the worker's free list."* This binds the group free
  path absolutely.
- **`.ai/canvas-threading.md` §7 says "There is no refcount"** — and that statement is
  about **textures**, whose ownership is the RES closed-flag model. A group is not a
  resource and has no closed flag; it can be referenced by several parents at once, so
  a count is the only way to know when the last reference goes. §4.3 reconciles the
  two: a refcount answers *"is it still referenced"*, and the existing frame-drain gate
  answers *"is a frame still drawing it"*. Both are required; neither replaces the
  other.

## 3. Design Overview

Five pieces:

1. **A process-global group table** — name → (buffer, revision, refcount, retired
   frame), living beside `CANVAS_SCENE_SYMBOL`. §4.1.
2. **`setGroup` / `removeGroup`** on the worker, the only thread that allocates or
   frees. §4.2.
3. **Lifetime** — refcount over scene and parent-group references, plus the existing
   frame-drain deferral before the free. §4.3.
4. **Resolution in `present`** — names → buffer handles, depth check, revision folding
   into the frame-skip comparison. §4.4.
5. **Software rendering** of a resolved group tree. §4.5.

**Where the correctness risk concentrates:** lifetime, and it is the memory-corruption
class. A group freed while the graphics thread is walking it is a use-after-free across
a thread boundary, in a subsystem whose whole design document exists because that class
of bug is easy to write here. It is scheduled **last** (Phase 5), behind the tests, and
the free path is a worker-only operation gated on both the refcount and the frame
counter.

**Where the design uncertainty concentrates:** the frame-skip interaction (§2, §4.4).
It is not in the feature request, it is easy to get subtly wrong, and a mistake shows
up as "sometimes the screen doesn't update" — the worst kind of bug to debug. Phase 1
pins it with a test before any of the machinery exists.

**Byte-identity is NOT this letter's gate.** **Expected NOT to diff:** every existing
golden — no existing scene has a `Group`. **Expected to diff:** `.ncodesum` on every
canvas-emitting target, `mfb man canvas` (two new members, one new type).

### Rejected alternatives

- **Flatten nested groups into the parent's buffer at `setGroup` time.** Rejected, and
  explicitly excluded by the feature request: it turns a shared child into N copies, so
  changing the child would require rebuilding every ancestor, and a cycle would not be
  detectable — it would hang at `setGroup` instead of raising at `present`.
- **Resolve names on the graphics thread.** Rejected: it needs the string table
  cross-thread and a lock around it, and `.ai/canvas-threading.md` §9 records that the
  design deliberately has no lock on the scene path.
- **Store groups in arena globals.** Rejected: §2's per-thread arena finding makes them
  invisible to the renderer. This is the exact mistake the threading document was
  written to prevent.
- **Free eagerly in `removeGroup`.** Rejected for the reason §3 of the threading
  document gives for the scene ring: the block being freed may be the one the renderer
  is reading right now.
- **A closed-flag model like `Image`'s, with no count.** Rejected: an image has exactly
  one owner (its `RES` binding); a group can be referenced by several parent groups and
  by the live scene simultaneously, so there is no single "closed" event that means "no
  longer referenced".

## 4. Detailed Design

### 4.1 The group table

A new process-global writable data symbol, `CANVAS_GROUPS_SYMBOL`, holding a fixed
array of `CANVAS_MAX_GROUPS` slots (recommend **256**). Each slot:

| Field | Meaning |
|---|---|
| `name` | pointer to the interned name string, or null for a free slot |
| `items` | pointer to the group's published item block array |
| `count` | how many items |
| `revision` | bumped on every `setGroup` for this name (§4.4) |
| `refs` | live references: scenes + parent groups (§4.3) |
| `retiredFrame` | frame stamp when the last reference dropped, or `-1` |

Fixed rather than growable because it is process-global storage the graphics thread
reads without a lock; a reallocating table would move under a reader. 256 slots is
generous for a named-sub-picture facility; `setGroup` past it raises, rather than
silently evicting.

**Size it deliberately (G4).** The two existing canvas tables are 80 bytes
(`CANVAS_SCENE_SLOTS * 8`) and 256 bytes (`CANVAS_FONT_SLOTS = 16` ×
`CANVAS_FONT_SLOT_BYTES = 16`), and both are emitted as literal zero bytes
(`value: "00".repeat(size)`) into **every** canvas binary. Six `u64` slots × 256
entries is 12,288 bytes — 48× the larger of the two — carried by a program that
installs no groups at all. That is a fraction of a percent of a real binary and is
almost certainly the right trade for a table that must not move under a reader, but it
should be a decision rather than a number inherited from this document: 64 slots
(3,072 bytes) is still generous for named sub-pictures. Whichever is chosen, say why
in the constant's doc comment.

### 4.2 `setGroup` and `removeGroup`

Both run on the worker, which is the only thread that may allocate or free
(`.ai/canvas-threading.md` §3).

**`setGroup(name, items)`:**

1. Deep-copy `items` into a fresh block with `CodeBuilder::copy_flat_block`, the
   primitive `emit_publish` itself calls for a scene — same guarantees
   (shrink-to-fit, so §3.1's content comparison holds), but **not** `emit_publish`
   itself, which writes the scene slot (**G2**).
2. Resolve each **nested** `Group` node's name to a slot index and **increment that
   slot's `refs`**. A nested name that does not resolve is left as a no-op node and
   takes no reference.
3. Find the slot for `name`, or claim a free one.
4. If the slot already had a buffer, **retire** it (do not free): decrement the refs
   its own nested children held, and stamp it for the frame-drain gate.
5. Publish the new pointers, then bump `revision` **last** — the same
   publish-then-revision ordering the scene ring uses (`.ai/canvas-threading.md` §3).

**`removeGroup(name)`:** clear the slot's name so no future `present` can resolve it,
decrement the refs its children hold, and drop the table's own reference. The buffer is
freed by §4.3's gate, not here. Removing an absent name is a no-op, not an error —
symmetric with a `Group` node naming an absent group.

### 4.3 Lifetime

A group's buffer may be freed when **both**:

- **`refs == 0`** — no published scene and no live parent group names it, and the
  table's own reference is gone (`removeGroup`, or a `setGroup` that replaced it); and
- **`retiredFrame < lastCompletedFrame`** — a frame has completed since the last
  reference dropped, so no render can still be walking it.

The second condition is **exactly the gate `.ai/canvas-threading.md` §7 specifies for
textures and §3 specifies for retired scene blocks**, reused deliberately so there is
one drain rule in the subsystem rather than three.

The free itself happens on the **worker**, on the next `present`. The graphics thread
never returns memory.

**But not where the scene ring reclaims (G7).** `emit_reclaim_retired` is emitted
*after* `builder.emit(abi::label(&publish))` in `gen_present.rs`, and the unchanged path
returns at the `skip` label above it — so the ring reclaims **only on a publish that
actually changes the scene**. Placing the group free "beside" it inherits that, and then
`removeGroup("panel")` followed by presents of an unchanged scene never frees the
buffer: the frame skip is working, so the reclaim never runs, and the memory is held
until something unrelated changes the scene.

The group free must therefore run **before** the content comparison, on every `present`,
not on the publish path. It is a scan of at most `CANVAS_MAX_GROUPS` slots with no
allocation, so it is cheap enough to be unconditional — and a memory bound that depends
on the scene changing is not a bound.

Phase 5's own race-matrix row *"the same, then a completed frame, then a `present`: the
buffer is freed exactly once and `groups=` drops by one"* is the test that catches this,
**provided its final `present` presents an unchanged scene.** Write it that way
deliberately; a test that changes the scene between the remove and the check passes on
the wrong implementation.

**Reference accounting, precisely:**

- The table holds **one** reference to each named group while the name is live.
- Each published scene holds one reference per **resolved** `Group` node in it.
- Each group's buffer holds one reference per **resolved** nested `Group` node in it.
- A scene being retired (§3 of the threading doc) drops its references at the same
  moment its block is reclaimed — not when it is displaced — so a group referenced only
  by the retired scene survives exactly as long as the scene does.

### 4.4 Resolution in `present`, and the frame skip

`__canvas_present` gains a resolution pass before `publishScene`:

1. **Walk the scene, depth-first, resolving every `Group` node's name to a slot
   index**, and recursing into each resolved group's own items to resolve *their*
   nested nodes. Track depth; at depth > **64**, **raise**.
2. An unresolved name is left as a no-op node — a silent no-op, per the spec, and
   distinct from the depth error.
3. Publish the scene with slot indices in place of names.

**The depth limit is also the cycle detector.** A cycle is unbounded depth, so it trips
at 64 with the same error. The message must say so — *"canvas group nesting exceeded 64
— this is a cycle or a bug"* — because a user who wrote a cycle and a user who wrote 65
honest levels need the same fix and would otherwise read the error very differently.
Depth is counted per *path*, not per node, so a diamond (two parents naming one child)
is legal and cheap.

**The frame skip (§2's problem).** The comparison `publishScene` makes must include,
for each resolved group node, that group's **current `revision`**. So `setGroup`
followed by `present` with an unchanged list republishes and redraws; `present` twice
with no `setGroup` between still skips.

**Not, however, "written into the published node" — there is nowhere to write it
(G8).** `Group` is `dx AS Float`, `dy AS Float`, `name AS String`; the published node is
a copy of that record and has no slot for a slot index or a revision, and adding two
fields for the renderer's use would put them on the user's constructor. Nor can the
`name` pointer carry the signal: two presents of the same scene reuse the same string,
so it compares equal precisely when the revision needs to say otherwise.

`publishScene` compares the **raw bytes of the `DrawItem` list's data region**
(`emit_compare_bytes_branch` in `gen_present.rs`, over `capacity * stride` bytes), so
anything the skip must see has to be inside those bytes or inside a second block
compared alongside them.

**Publish a parallel resolved-groups signature and compare it too** — a
`List OF Integer` of `(slotIndex, revision)` pairs, one pair per resolved group node in
scene order, built by the §4.4 walk. `publishScene` compares the items *and* the
signature, publishing both or neither. A scene with no groups gets an empty signature
and compares exactly as it does today, so nothing that does not use groups changes.

Note this cannot reuse `publishHashes`, which looks like the same shape: `__canvas_present`
calls `publishScene` first and `publishHashes` only inside the `IF`, so the hashes are
written *after* the skip has already been decided (`sed -n 88,92p func_present.rs`).

This also means **`setGroup` alone does not repaint** — it takes effect at the next
`present`. That is the right semantics (it matches `present` being the install point
for everything else) and it must be documented, because the natural expectation is the
opposite.

### 4.5 Software rendering

`__canvas_renderScene` (`helper_render.rs:28`) walks the published items. A resolved
`Group` node pushes `(dx, dy)` onto an accumulated offset, walks the group's items, and
pops.

Because every distance field is evaluated in **surface coordinates**, a translated
group is drawn by offsetting the item's **bounds** and evaluating the distance at
`p - offset`. That is precisely the mechanism plan-116-C established for `transform`
(evaluate the shape at the inverse-transformed query point), specialised to a
translation — so no distance function changes here either.

The accumulated offset is applied **before** the surface clamp: the item's bounds are
offset first, then clamped to the surface, or a group translated partly off-screen
would be clipped against the wrong rectangle. (This is the "clamp the quad to the
surface *after* offsetting" rule from the feature request, stated for the CPU path.)

**Two per-item things are in SURFACE coordinates and must be handled explicitly, in
opposite directions (G5).** "Evaluate the distance at `p - offset`" covers every
distance function, the coverage rule and the stroke band, and covers nothing else:

* **`Paint.clip` stays at `p`.** It is a surface rectangle by definition
  (plan-116-B §Non-goals), so a group's translation moves the shape and not the clip.
* **A `Text` item's glyph sampling must move to `p - offset`.** A glyph is a cached
  coverage bitmap indexed by whole pixels from the run's origin, not a distance field,
  so it is on a separate path from "evaluate the distance at `p - offset`" and a fix
  written for distances misses it. Text in a translated group would sample the wrong
  texels.
* **`Paint.fillGradient` is an open decision, not a defect — settle it in Phase 4.**
  plan-116-F evaluates the ramp at the surface point (`px`/`py` against
  `gradFX`/`gradFY`, `sed -n 342,350p` and `:500,510p` of `helper_items.rs`) against an
  axis read straight from the record with **no transform applied**. So a gradient is
  surface-anchored today and `Paint.transform` does not drag it — which
  `06_canvas.md` states deliberately ("rather than being dragged around by it").

  A group offset could therefore go either way, and the two answers are both defensible:
  leave it surface-anchored, consistent with `Paint.transform` and with the clip; or
  move it, consistent with a group being a self-contained sub-picture.

  **Recommend moving it**, on the letter's own stated goal: §1 records the user's goal
  (1) as *reuse* — "a sub-picture authored once and referenced from many scenes and
  positions". A gradient-filled item that renders differently at every position is not
  reusable, and a group is the one construct here whose entire purpose is to be drawn
  somewhere else. `Paint.transform` is a different thing — it reshapes one item in
  place — so following it is not obviously the consistent choice.

  Whichever is chosen: **write it in `06_canvas.md`, and pin it with the diamond test**,
  which is the scene that can tell the two apart.

Phase 4 must do this and test it, and the reason it matters *here* rather than in
plan-116-H is that this file defines the **oracle**. H compares both GPUs against it. A
wrong software answer is not caught by that comparison — it is *ratified* by it.

The geometry cache is unaffected: a group's items are cached by their own geometry, and
the offset is applied at draw time, so the same group drawn at two offsets hits one
cache entry. That is the main performance reason to have groups at all and it should be
asserted (`MFB_CANVAS_STATS`'s `entries=`).

### 4.6 Damage: a group node must damage what it draws

`__canvas_damageFor` diffs the frame against `__CANVAS_LAST_HASHES` /
`__CANVAS_LAST_BOUNDS` — per-item hashes and per-item *geometry bounds*
(`helper_damage.rs`; the bounds come from each item's header). A `Group` node's own
geometry is the empty `NONE` header, whose bounds are zero — so without care, a
`setGroup` that changes what is drawn would hash as changed (the revision is in the
published node, §4.4) but damage a zero-area rectangle, and the partial-redraw path
would clear and repaint **nothing** where the group actually renders. The failure
reads as "the screen updates only on full redraws", the §3 class of bug.

The rule: a resolved group node's recorded bounds are **the axis-aligned hull of
its resolved children's bounds (recursively), offset by the node's accumulated
`(dx, dy)`** — computed during the §4.4 resolution pass, where the walk already
visits every child. A diamond contributes its hull once per reference, at each
reference's offset. This keeps a `setGroup` update's damage exactly the area the
group can touch, and a moved node (`dx`/`dy` change) damages both the old and new
hulls the same way any moved item does — through the hash-change diff on bounds.

## Compatibility / Format Impact

- **BREAKING: the `DrawItem` union gains a tenth variant.** A user's exhaustive
  `MATCH` stops compiling until it handles `Group`.
- **`canvas::setGroup`, `canvas::removeGroup`, `canvas::Group` are new exported
  surface.**
- **`present` gains a new raisable error** for depth exhaustion. It must be a
  trappable named error, listed in the member's `errors:` vec and in
  `src/docs/spec/diagnostics/02_error-codes.md`.
- **No existing scene changes.**
- **`.ncodesum` churn** on every canvas-emitting target.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the
> work; `- [~]` for partial with a one-line remainder; fill `Commit:` on landing.
> **An unticked box means NOT DONE.**

### Phase 1 — Pin the frame-skip semantics before building anything

The design's least obvious requirement (§2, §4.4), written as a failing test first.

- [ ] Add a `#[ignore]`d test to `tests/rt_canvas_graphics_thread.rs` that, with
      `MFB_CANVAS_SYNC=1`, does: `setGroup(A)` → `present([Group A])` → `setGroup(A')`
      → `present([Group A])` (identical list) → asserts **two** frames were rendered
      and the second shows `A'`.
- [ ] Add its sibling: `present` twice with no `setGroup` between → **one** frame.
- [ ] Leave both `#[ignore]`d with a comment naming the phase that un-ignores them.

Acceptance: both tests exist and are ignored, and the design section they test (§4.4)
names them. This phase ships no behaviour; it ships the definition of done for Phase 4.
Commit: —

### Phase 2 — The type, the two members, and the two test amendments

The whole breaking surface change, with nothing yet reading it.

- [ ] Add the `Group` record (`dx`, `dy`, `name`) and append `"Group"` **last** to the
      `DrawItem` union.
- [ ] Amend `draw_item_variant_set_is_frozen` (append `"Group"`; extend the doc comment
      to name plan-116-G alongside plan-116-E).
- [ ] Narrow `every_draw_item_variant_carries_a_paint` (`mod.rs:1161`) to exempt an
      explicit container list `["Group"]`, per §2(a). **Do not weaken the assertion for
      the other nine.**
- [ ] Register `setGroup` and `removeGroup` as public members with full `intro`/`desc`/
      `example`; add both to `MEMBERS` in
      `tests/cli_canvas_man_examples_compile.rs`.
- [ ] Add a `Group` arm to **all seven** exhaustive `MATCH item` sites in
      `helper_geometry.rs`, not the two this letter originally named (**G6**):
      `__canvas_headerFor` and `__canvas_tailFor` (both returning
      `__canvas_emptyHeader()` / an empty tail — a group has no geometry of its own),
      and also `__canvas_tailMatches`, `__canvas_headerIsDeferred`,
      `__canvas_deferredHeader`, `__canvas_deferredHash` and `__canvas_hashItem`.
      Re-measure first: `grep -n 'MATCH item' src/codegen/builtins/canvas/helper_geometry.rs`.
- [ ] `__canvas_headerIsDeferred` returns **TRUE** for `Group`, and
      `__canvas_deferredHash` hashes its **name** and `dx`/`dy` (**G6**). A group has an
      empty header, and a non-deferred kind with an empty header makes every group in a
      scene share one geometry-cache entry — plan-98-G Correction 14, reproduced.
- [ ] Tests: `tests/cli_canvas_package.rs` constructs a `Group` and calls both members;
      `mfb man canvas types` lists `Group`.

Acceptance: `cargo test --no-fail-fast` green, every canvas golden byte-identical, and
a scene containing a `Group` compiles and **draws nothing** (the members are registered
but inert).
Commit: —

### Phase 3 — The group table and the two members' bodies

- [ ] Add `CANVAS_GROUPS_SYMBOL` and `CANVAS_MAX_GROUPS = 256` as process-global
      storage beside `CANVAS_SCENE_SYMBOL`, with the §4.1 slot layout.
- [ ] Implement `setGroup`'s deep copy by calling
      `CodeBuilder::copy_flat_block` — the primitive `emit_publish` itself calls —
      rather than writing a second copy or calling `emit_publish`, which is bound to
      the scene slot (**G2**).
- [ ] Implement `removeGroup` as a name clear + reference drop; **no free yet**.
- [ ] Extend `MFB_CANVAS_STATS` with `groups=` and `groupBytes=` — moved here from
      Phase 5, because this phase's own acceptance asks to *measure* the leak and this
      is the instrument that measures it (**G10**). It is also the only window onto
      worker-owned state a test has (`.ai/canvas-threading.md` §11).
- [ ] `setGroup` past `CANVAS_MAX_GROUPS` raises a named, trappable error. This one
      **is** new surface — no existing `7-705-00xx` constant means "a fixed table is
      full" — so mint it, and grep the *literal code* for collisions at that moment
      (`grep -rn '7705002[0-9]\|7705003[0-9]' src/ | grep -v docs/`), not the name
      (**G3**).
- [ ] Tests: `tests/rt_canvas_present_deep_copy.rs` gains a group case — mutate the
      list the caller passed to `setGroup` and assert the installed group is unchanged.
      `removeGroup` of an absent name is a no-op.

Acceptance: the deep-copy and absent-name cases pass; nothing is freed yet, so this
phase can leak by construction — assert that it does with `groups=`/`groupBytes=` (added
in this phase, **G10**), so Phase 5's gate has a measurable "before".
Commit: —

### Phase 4 — Resolution, depth limit, frame skip, software rendering

- [ ] The resolution pass in `__canvas_present` per §4.4: names → slot indices,
      depth-first, depth > 64 raises.
- [ ] The depth error **reuses `ErrDepthExceeded` (`77050024`)** rather than minting a
      new code — it already means exactly this (**G3**) — with the §4.4 message, listed
      in `present`'s `errors:`, and `02_error-codes.md`'s row extended to name group
      nesting alongside `json::parse`. Re-check the code is still that constant before
      you rely on it; codes race between sessions and grepping the *name* never proves
      the *code*.
- [ ] Fold each resolved group's `revision` into a **parallel signature block** that
      `publishScene` compares alongside the items — not into the published node, which
      has no room for it (§4.4, **G8**). An empty signature for a group-free scene must
      compare exactly as today.
- [ ] `__canvas_renderScene` walks resolved groups with an accumulated offset,
      offsetting bounds **before** the surface clamp (§4.5).
- [ ] The **glyph sampling** is evaluated at `p - offset` and the **clip** at `p`
      (§4.5, **G5**). Enumerate the positional reads rather than reasoning from the
      distance path, which is what hid the glyph case.
- [ ] **Decide** whether `Paint.fillGradient` follows the group offset (§4.5,
      **G5**), implement the decision in the oracle, document it in
      `06_canvas.md`, and pin it with the diamond test. Recommended: it follows.
- [ ] The resolution pass records each group node's damage bounds as its resolved
      children's offset hull (§4.6), so `__canvas_damageFor` sees real rectangles
      for group nodes.
- [ ] Both `*Renderable` predicates **decline any scene containing a `Group`** — the
      GPU cannot draw one until plan-116-H, and a predicate that accepted a kind its
      shader does not know is the exact failure `.ai/canvas-threading.md` §10 records
      as having happened.
- [ ] Un-ignore Phase 1's two tests.
- [ ] Tests: a nested group renders at the composed offset; a diamond (two parents, one
      child) renders twice and is legal; a 65-deep chain raises; a self-referencing
      group raises with the same error; a `Group` naming an absent group draws nothing
      and does **not** raise; a group drawn at two offsets produces **one** geometry
      cache entry (`MFB_CANVAS_STATS` `entries=`).
- [ ] Tests for **G5**: a **gradient-filled** item in a group drawn at `(0,0)` and the
      same group drawn at `(37, 53)` are the same picture translated — the diamond form
      is the sharp one, because it proves the ramp followed the shape rather than the
      buffer. And a **clipped** item in a translated group keeps its clip where the
      surface rectangle is, not where the group moved to.
- [ ] Damage tests (`MFB_CANVAS_DAMAGE=1`, in `tests/rt_canvas_damage.rs`):
      `setGroup(A')` then an identical `present` yields a **partial** frame whose
      damage rectangle covers the group's drawn area (assert via the stats
      `damage=` field AND a repainted pixel inside the group, far from any other
      item); changing only a node's `dx` repaints both the old and new positions.

Acceptance: Phase 1's two frame-skip tests pass un-ignored; all six behavioural cases
pass; every existing golden is byte-identical; a scene containing a `Group` is provably
declined by both GPU predicates (assert via `MFB_CANVAS_STATS`, not by pixel equality).
Commit: —

### Phase 5 — Lifetime: the refcount and the drain gate (largest blast radius)

Memory-correctness, landed last, behind every test above.

- [ ] Implement the reference accounting of §4.3 exactly: table reference, per-scene
      references, per-parent-group references, and the drop-on-scene-reclaim rule.
- [ ] Implement the free gate — `refs == 0 AND retiredFrame < lastCompletedFrame` —
      executed on the **worker**, at the top of `present` and **before the content
      comparison**, not beside `emit_reclaim_retired`, which only runs on the publish
      path (**G7**).
- [ ] **Build the mid-frame affordance the race rows need, or state them as
      probabilistic (G9).** §11 has four test affordances and none holds the graphics
      thread mid-frame; the only "mid-render" rows proven today (R5, R7) get there
      through `MFB_CANVAS_RESIZE_W`/`_H`, which is resize-specific, and R1 — the row
      closest to these — is marked *not yet reachable*. Decide this before writing the
      matrix below, because a row tested by luck reports the same green as one tested by
      construction.
- [ ] Tests, as a race matrix in the style of `.ai/canvas-threading.md` §8 — add the
      rows to that document too:
      - `present([Group A])` → `removeGroup(A)` → graphics mid-frame: the in-flight
        frame completes normally. **This is the row that needs G9's decision** — with
        `MFB_CANVAS_SYNC` off, `present` returns before the frame is drawn and the
        worker can reach `removeGroup` while it is in flight, but nothing guarantees it
        does.
      - the same, then a completed frame, then a `present` **of an unchanged scene**:
        the buffer is freed exactly once and `groups=` drops by one. The "unchanged" is
        the whole point of the row (**G7**) — a scene that changes takes the publish
        path and would pass against a free placed where the ring's reclaim is.
      - `removeGroup(A)` while a **parent group** still names A: not freed.
      - `setGroup(A, …)` replacing a live A: the old buffer is freed only after a
        frame completes.
      - program exits with a group referenced by an in-flight frame: no use-after-free
        (the R12 row's group analogue).
      - 200 × `setGroup`/`removeGroup` in a loop: `groupBytes=` returns to its
        starting value.

Acceptance: all six race-matrix rows pass; the 200-iteration loop shows no growth in
`groupBytes=`; `cargo test --no-fail-fast` green on mac+RELEASE **and** linux+DEBUG
with `--no-fail-fast` (a failing earlier test silently skips every later `rt_*`).
Commit: —

### Phase 6 — Docs and gates

- [ ] `mod.rs` — `Group`, `setGroup`, `removeGroup` descriptions and examples. State:
      a nested group stays a reference; `setGroup` takes effect at the next `present`;
      an absent name is a silent no-op; the depth limit is 64 and exceeding it raises;
      the group is translated, not transformed.
- [ ] **No memory vocabulary on any of it** — no "own", "free", "refcount", "release".
      Say what a developer observes: *"the group stays installed until you replace or
      remove it"*. `scripts/man-census.sh --memory-scope` → 0 unclassified hits.
- [ ] `src/docs/spec/app/06_canvas.md` — a groups section: the naming model, the
      translation, the nesting limit, the no-op rule, and the `present`-is-the-install-
      point rule.
- [ ] `.ai/canvas-threading.md` — a new section for the group table (process-global,
      worker-owned, refcount **plus** the existing drain gate, and why §7's "there is
      no refcount" is about textures and still true of them), and the new race-matrix
      rows from Phase 5.
- [ ] `src/docs/spec/diagnostics/02_error-codes.md` — **one** new error (the
      table-full one), plus an extension of `ErrDepthExceeded`'s existing row to
      name group nesting alongside `json::parse` (**G3** — the depth error is not
      new).
- [ ] `scripts/man-run-examples.sh canvas --run` passes.
- [ ] `scripts/regen-ncodesum.sh`. Expect **0 diffs, and do not read that as
      evidence**: `ls tests/byte-identity/` has no `canvas` directory and no
      fixture there imports it, so the ncodesum gate is silent about this package
      (plan-116-F **F11**). The gates that are evidence for this letter are the
      canvas rt tests and the golden harness.

Acceptance: `cargo test --no-fail-fast` green on **mac RELEASE, mac DEBUG
(`--bin mfb`, the only run anywhere that executes the `debug_assert!`s — plan-116-E
**E6**) and box 2228 RELEASE**, `scripts/test-accept.sh` green (read its `N ran` —
`cargo test`'s copy of the corpus skips 519 `syntax/` fixtures, plan-116-F **F13**),
`scripts/artifact-gate.sh all` 0 diffs, and `mfb man canvas setGroup` /
`removeGroup` / `mfb man canvas types` render correct, example-backed pages.
Commit: —

## Validation Plan

- **Tests:** `tests/rt_canvas_graphics_thread.rs` (frame skip ×2, race matrix ×6),
  `tests/rt_canvas_rasteriser.rs` (offset composition, diamond, cache sharing),
  `tests/rt_canvas_present_deep_copy.rs` (group deep copy),
  `tests/cli_canvas_package.rs` (surface). Negative cases: absent name (no-op, no
  raise); 65-deep chain (raise); self-reference (raise); table full (raise).
- **Coverage check:** the group table is emitter/runtime code reached only when a
  canvas program is built — confirm the new lines are in the denominator with
  `cargo llvm-cov --bin mfb`. The MFBASIC-side resolution walk is invisible to it;
  its coverage is the rt cases.
- **Runtime proof:** a program that installs a group, presents it at three offsets,
  replaces it, removes it, and loops 200 times — with `MFB_CANVAS_STATS` showing
  `groups=` and `groupBytes=` returning to baseline, and `MFB_CANVAS_DUMP` frames
  matching a flat scene drawn the same way.
- **Doc sync:** `src/docs/spec/app/06_canvas.md`,
  `src/docs/spec/diagnostics/02_error-codes.md`, `.ai/canvas-threading.md`, and the
  `mod.rs` descriptions.
- **Acceptance:** `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`, `rustup run 1.96.0 cargo fmt --all &&
  (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **`CANVAS_MAX_GROUPS = 256`, fixed (§4.1).** Recommended: a growable table would
  move under a lock-free reader on the graphics thread. Raising the number is cheap;
  making it dynamic is a different design.
- **`setGroup` takes effect at the next `present` (§4.4).** Recommended — it matches
  `present` being the install point for everything else, and the alternative
  (`setGroup` signals a redraw itself) would repaint for a group no scene draws, which
  is the mistake `.ai/canvas-threading.md` §4 trigger 5 exists to avoid.
- **One error for both cycles and honest over-nesting (§4.4).** Recommended: they are
  indistinguishable without a full cycle search, and the fix is the same. The message
  must name both possibilities.
- **Whether a `Group`'s `dx`/`dy` should instead be a full `Transform`.** Recommend
  **no** — a translation is what was specified, it composes by addition (so the
  accumulated offset is two floats, not a matrix chain), and `Paint.transform` on the
  group's items covers the rest.

## Corrections

**G10 (2026-09-03, pre-execution) — Phase 3's acceptance needs an instrument Phase 5
builds.** Phase 3 ends *"this phase can leak by construction — assert that it does, so
Phase 5's gate has a measurable 'before'"*, and the thing that makes a group leak
assertable — `groups=` and `groupBytes=` on `MFB_CANVAS_STATS` — was a Phase 5 task.
The group table is worker-owned, and `.ai/canvas-threading.md` §11 is explicit that the
stats line is the *only* window a test has onto state like that, so there is no
substitute available in Phase 3.

Left alone this resolves itself the wrong way: Phase 3's acceptance is unmeasurable, so
it gets waved through, and Phase 5 then has no "before" to compare its gate against —
which is exactly what that sentence was written to prevent. Moved the stats task to
Phase 3.

It costs Phase 5 nothing: the counters are two more fields on a line that is already
being written, and having them two phases earlier means the deep-copy and replace paths
are observable while they are being built rather than only after the free lands.

**G9 (2026-09-03, pre-execution) — Phase 5's race matrix asks for "graphics mid-frame"
and no affordance produces it.** `.ai/canvas-threading.md` §11 lists exactly four test
affordances — `MFB_CANVAS_RESIZE_W`/`_H`, `MFB_CANVAS_DUMP`, `MFB_CANVAS_STATS`,
`MFB_CANVAS_GLYPH_BUDGET` — and none holds the graphics thread inside a frame. §8's two
"mid-render" rows that *are* proven (R5 present-during-render, R7 resize-during-render)
reach it through `MFB_CANVAS_RESIZE_W`/`_H` firing after the first completed frame while
the worker sits in `os::sleep`, which is specific to resize. R1 — *"present →
`destroyImage` → graphics mid-record"*, the row structurally identical to the one this
phase adds — is marked **not yet reachable**.

Phase 5 is the memory-corruption phase, so this is the phase where "tested by luck"
matters most. Two honest ways forward, and the letter should pick one rather than
discover the problem while writing the test:

* **Build the affordance.** A `MFB_CANVAS_FRAME_HOLD_MS` that makes the graphics thread
  sleep at a fixed point inside a frame turns every one of these rows deterministic, and
  would retroactively make R1 reachable — which is worth more than this letter. It is
  off by default and off the production path, like the other four.
* **State them as probabilistic** and run the scene N times, asserting no crash and a
  correct final frame. Weaker, and it must *say* it is weaker in the test's own comment,
  or the next reader takes a green run as proof of the ordering.

The skill's rule applies squarely: a provability gap is a missing prerequisite, not a
reason to write a weaker test quietly.

**G8 (2026-09-03, pre-execution) — §4.4's revision has nowhere to live.** The section
says *"the resolution pass writes `(slotIndex, revision)` into the published node"*.
The published node is a copy of a `Group` record, and §Goal defines that record as
`dx AS Float`, `dy AS Float`, `name AS String` — three fields, no spare words. Adding
two would put renderer bookkeeping on the user's constructor, which MFBASIC named
construction makes mandatory to supply.

Nor can the existing bytes carry it. `publishScene` compares the raw data region of the
`DrawItem` list (`emit_compare_bytes_branch`, `sed -n 200,228p
src/codegen/builtins/canvas/gen_present.rs`), and a `Group` node's bytes are its two
floats and a string **pointer** — the same pointer on every present of the same scene,
which is exactly when the revision needs to differ.

Recommended and written into §4.4: publish a parallel `List OF Integer` signature of
`(slotIndex, revision)` pairs and have `publishScene` compare it alongside the items,
publishing both or neither. A group-free scene gets an empty signature and behaves
byte-for-byte as today, which is what keeps this letter's "no existing golden may move"
non-goal true.

The trap next to it: `publishHashes` looks like the block to reuse and is not.
`__canvas_present` calls `publishScene` first and `publishHashes` only *inside* the
resulting `IF`, so the hashes are written after the skip has been decided — a revision
folded in there is read one present too late, which presents as "the screen updates on
the present *after* the one that should have updated it". That is the §2 bug wearing a
different hat.

**G7 (2026-09-03, pre-execution) — "beside the scene ring's own reclaim" is the one
place the group free must not go.** §4.3 and Phase 5 both site the free "at the top of
`present`, beside the scene ring's own reclaim". Measured: `sed -n 218,254p
src/codegen/builtins/canvas/gen_present.rs` shows `emit_compare_bytes_branch` sending
the unchanged case to a `skip` label that sets `RESULT_OK_TAG` and **returns**, with
`emit_reclaim_retired` emitted after the `publish` label below it. The ring therefore
reclaims only when the scene content changed.

That is right for the ring — it is reclaiming a block *displaced by this publish*, so
there is nothing to reclaim when nothing was published. It is wrong for groups, whose
lifetime is driven by `setGroup`/`removeGroup`, calls that change no scene item at all.
`removeGroup("panel")` on a static scene would then hold the buffer forever: the frame
skip works, so the reclaim never runs.

The fix is to site the group free before the comparison rather than beside the ring's
reclaim, and it is cheap enough to be unconditional — a scan of at most
`CANVAS_MAX_GROUPS` slots, no allocation. Both §4.3 and the Phase 5 task now say so.

Phase 5's race-matrix row already describes the catching test, but only if its final
`present` is of an **unchanged** scene; as written that was unspecified, and the
natural way to write the test — change something so a frame renders — is exactly the
way that passes against the wrong placement. The row now says so.

**G6 (2026-09-03, pre-execution) — a new `DrawItem` variant touches seven exhaustive
`MATCH` sites, not two.** Phase 2 says *"Add the `Group` arms to `__canvas_headerFor`
and `__canvas_tailFor`"*. Measured:
`grep -n 'MATCH item' src/codegen/builtins/canvas/helper_geometry.rs` → **seven**, at
`:142` `__canvas_headerFor`, `:595` `__canvas_tailFor`, `:810` `__canvas_tailMatches`,
`:961` `__canvas_headerIsDeferred`, `:984` `__canvas_deferredHeader`, `:1022`
`__canvas_deferredHash`, `:1080` `__canvas_hashItem`. `grep -n Ellipse` on the same file
shows plan-116-E's variant in all seven (`CASE Ellipse(e)` at `:159, :615, :835, :978,
:1001, :1039, :1105`) — so the letter immediately before this one already paid this cost
and the count is not a guess about the future.

MFBASIC `MATCH` over a union is exhaustive, so the missing arms fail to compile and the
task self-corrects — cheap. The count is not the finding. **The finding is that a
`Group` must be a DEFERRED kind**, and the letter does not say so.

`__canvas_hashItem`'s own comment (`sed -n 1063,1073p`) states the rule and the scar:

> *"a deferred kind probes the geometry cache on the hash alone. Hashing that empty
> header would therefore give every string on screen one hash, and the cache would hand
> all of them the first string's glyph run: a sixty-item scene drew one glyph, sixty
> times, in one place (plan-98-G Correction 14)."*

Phase 2 gives `Group` `__canvas_emptyHeader()`. Every group in a scene would then hash
identically, and `__canvas_geometryFor` would hand them all the first group's cache
entry — a scene of five different groups drawing the first one five times, which is
plan-98-G Correction 14 exactly, reproduced by following this letter as written.

`Text` is the only kind that answers `TRUE` from `__canvas_headerIsDeferred` today
(`sed -n 961,981p`), and the deferred path exists precisely so a kind with no header of
its own can carry by hand what the header would have carried. `Group` is the second such
kind. So: `__canvas_headerIsDeferred` returns `TRUE`, and `__canvas_deferredHash` gets an
arm hashing the **name** and `dx`/`dy` — the name above all, since two groups at the same
offset differ only by it.

**This is separate from §4.4's revision, and the two must not be confused.**
`__canvas_present` calls `canvas::publishScene(items)` and only *then*
`publishHashes(__canvas_hashScene(items))` (`sed -n 88,92p func_present.rs`) — the skip
is decided before any hash is published, so the frame-skip revision belongs in the
published node exactly as §4.4 says, and the hash is the *geometry cache* key. Putting
the revision in the hash as well is defensible (a `setGroup` should invalidate the cached
geometry too) but it does not substitute for §4.4, and §4.4 does not substitute for this.

**G5 (2026-09-03, pre-execution) — the software group path would slide every gradient,
and this file is the oracle, so nothing downstream could catch it.** §4.5 says a
translated group is drawn "by offsetting the item's bounds and evaluating the distance
at `p - offset`", and concludes that no distance function changes. True, and not
sufficient: two per-item things are positional and are *not* distance functions.

`Paint.clip` is a surface rectangle (plan-116-B) and must **not** move with the group.
A **`Text` item's glyph sampling** must: a glyph is a cached bitmap indexed by whole
pixels from the run's origin, not a distance field, so it sits on a different path from
"evaluate the distance at `p - offset`" and a fix written for distances misses it
entirely — text in a translated group would sample shifted texels, and blank ones once
the offset exceeds the glyph.

**`Paint.fillGradient` is a decision, and an earlier draft of this correction asserted
it was a bug.** It is not: `sed -n 342,350p src/codegen/builtins/canvas/helper_items.rs`
shows the axis read straight from the geometry record with no transform applied, so a
gradient is surface-anchored and `Paint.transform` does not drag it either — which
`06_canvas.md` states on purpose. A group offset could consistently follow either
convention, so §4.5 now poses it as a decision with a recommendation rather than a
fix.

The reason this is a G defect and not an H one, even though H is the letter that touches
shaders: **this file defines the oracle.** plan-116-H's whole acceptance is "both GPUs
match the software oracle within `Tolerance::GPU_DEFAULT`". If the oracle slides a
group's gradient and both shaders are then written to match it, every comparison in H
passes and the picture is wrong on all three renderers. A wrong oracle is not caught by
comparison against the oracle; it is ratified by it.

Recorded as a Phase 4 task and a Phase 4 test, with plan-116-H's **H2** carrying the
same fix for the two shaders. The diamond is the test that can see it — one group, two
offsets, one buffer, and the two draws must be the same picture translated.

**G4 (2026-09-03, pre-execution) — there are three process-global canvas symbols, not
one, and the third is the precedent this letter needed.** §2's table says *"Process-
global canvas state symbols today — 1 (`CANVAS_SCENE_SYMBOL`)"*. Measured with
`sed -n 906,932p src/codegen/engine/builder/mod.rs`, the `module_uses_canvas(module)`
arm pushes **three**:

* `graphics_state_data_object()` (`src/codegen/runtime/canvas/mod.rs`),
* `CANVAS_SCENE_SYMBOL` = `_mfb_rt_canvas_scene`, and
* `CANVAS_FONTS_SYMBOL` = `_mfb_rt_canvas_fonts` — added by plan-98-G with the comment
  *"the loaded-font table, process-global for the same reason — the worker loads a font
  and the graphics thread rasterises from it."*

Both constants live in `src/codegen/error/constants/error_constants.rs:404,:422`.

This is not a nitpick, it changes what §4.1 has to argue. As written, §4.1 justifies a
process-global group table from first principles (arena state is per-thread, so a
reader on the graphics thread cannot see worker arena memory) as though it were the
second such table ever. It is the **fourth**, and `CANVAS_FONTS_SYMBOL` is a
name-keyed, worker-written, graphics-read, fixed-size table — structurally the same
object as the group table, added for the same stated reason.

So the group table should be built as `CANVAS_FONTS_SYMBOL`'s sibling, and Phase 3
should read that declaration before writing a new one. A design that re-derives an
existing pattern usually diverges from it in some small way, and here the small ways
(fixed size, no reallocation, worker-only writes) are the ones that keep a reader on
another thread safe.

Corrected in the table. §4.1's *conclusion* is right and unchanged; only its claim to
novelty was wrong.

**G3 (2026-09-03, pre-execution) — the depth error already exists; only the table-full
one is new.** Phase 4 asks for "a named trappable error" for group nesting past 64, and
Phase 3 for one when `setGroup` exceeds `CANVAS_MAX_GROUPS`, without saying whether
either is new. Measured:
`grep -rn '7705002[0-9]' src/ | grep -v docs/` shows the canvas block runs `77050020`
`ErrWrongMode` … `77050025` `ErrInvalidSurrogate`, and `77050024` is **`ErrDepthExceeded`**,
declared in `src/codegen/builtins/errorcode/mod.rs` as *"Structural nesting exceeds the
implementation depth limit… (`json::parse` stops at 256)"*.

That is this error, not a near-miss: a group cycle is unbounded structural nesting, and
a user who traps `ErrDepthExceeded` around a parser and around a `present` is asking
the same question both times. Minting a second depth code would split one concept
across two constants for no behavioural gain — and §4.4's own argument, that a cycle
and 65 honest levels need the same message, is the same argument one code down.

The table-full error genuinely is new: nothing in `7-705-00xx` means "a fixed-size
table is full".

Both tasks now say which, and both carry the warning the project memory records: a rule
or error code can be claimed by a peer session between this measurement and the phase
that uses it, and grepping the *name* never proves the *code* is free. Re-run the
literal-code grep at the moment of minting.

**G2 (2026-09-03, pre-execution) — `emit_publish` cannot be reused for `setGroup`;
the copy underneath it can.** Phase 3 says to implement `setGroup`'s deep copy "by
reusing `gen_present.rs:emit_publish`'s copy", which reads as *call `emit_publish`*.
It is not callable for a group: `emit_publish` opens with `let scene =
scene_base(builder)` and is parameterised by `SceneShape`, whose `slots()` and `tag()`
name the scene ring's own two pointer/count pairs
(`sed -n 85,101p src/codegen/builtins/canvas/gen_present.rs`). Publishing a *group*
into the scene slot is precisely the bug that phrasing invites.

What is reusable is the line `emit_publish` delegates the actual copy to —
`builder.copy_flat_block(&list_type, &incoming)`, at
`src/codegen/collection/layout/builder_collection_layout.rs:383`
(`grep -rn "fn copy_flat_block" src/`). It takes a type and a source and returns a
fresh pointer, with no opinion about where the result is stored, and for a collection
it routes to `copy_collection_tight` — the shrink-to-fit copy that §3.1's content
comparison depends on. That is the correct seam, and it is why Phase 3's task now
names it.

Recorded pre-execution rather than discovered mid-phase because the failure mode is
not a compile error: `emit_publish` would build and would install the group's items as
*the scene*.

**G1 (2026-09-03, pre-execution) — every line number this letter cites into
`src/codegen/builtins/canvas/mod.rs` was stale before a single task ran.** The letter
names `mod.rs:884`, `:913` and `:935` for the three tests that pin or iterate the
`DrawItem` variant list; they are at `:1109`, `:1139` and `:1161`. plan-116-F added
`GradientKind`, `GradientStop` and `Gradient` to that file between the letter being
written and being executed, moving everything below them.

Measured with `grep -n "fn draw_item_variant_set_is_frozen\|fn every_draw_item_variant"
src/codegen/builtins/canvas/mod.rs`. The counts the citations *support* are all still
right — 9 variants, 3 tests — so this is a navigation defect and not a scoping one,
which is exactly why it is worth fixing rather than shrugging at: Phase 2 tells its
executor to edit "`mod.rs:935`", and at `:935` today sits unrelated code that would
look plausible enough to edit.

Corrected in place in §2 and in Phase 2. The general lesson for G–J: cite a **symbol**
and the command that finds it, not a line — every letter of this plan edits `mod.rs`,
so every line citation into it decays the moment the letter before it lands.

- **C1 (2026-09-01, review — pre-execution).** Refreshed against plan-114 landing
  (all five letters archived; `2-203-0084` retired at `src/rules/table.rs:1015`) and
  against the series renumber (RES migration = plan-116-I, ownership = plan-116-J).
  Added §4.6: the damage diff keys on per-item geometry bounds, and a group node's
  own bounds are empty — without the resolved-hull rule a `setGroup` would repaint
  nothing on the partial-redraw path. Recorded the feature's two goals (reuse;
  CPU/GPU speed) from the user, 2026-09-01.

## Summary

Two things in this letter are not in the feature request and both would ship it
broken. The first is the frame skip: `setGroup` changes what is drawn without changing
the scene list, so without a revision folded into the content comparison a program
would call `setGroup` and see nothing happen — which is why Phase 1 writes that test
before anything else exists. The second is that a group is the subsystem's first piece
of shared state that can be referenced from more than one place, so the texture model's
"there is no refcount" does not carry over; it needs a count *and* the existing
frame-drain gate, and the free must stay on the worker because an arena is per-thread.
That lifetime work is scheduled last, behind every behavioural test, because it is the
only part of plan-116 whose failure mode is memory corruption rather than a wrong
pixel. Untouched by this letter: the GPU backends (plan-116-H), the `RES` migration
(plan-116-I), and resource ownership (plan-116-J).

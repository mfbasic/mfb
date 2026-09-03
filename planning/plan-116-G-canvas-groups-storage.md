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
`Text` through `FontRef` (`mod.rs:398`, `:412`) — the migration to direct `RES`
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
| Process-global canvas state symbols today | 1 (`CANVAS_SCENE_SYMBOL`) | `.ai/canvas-threading.md` §3 |
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
> members.
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

### 4.2 `setGroup` and `removeGroup`

Both run on the worker, which is the only thread that may allocate or free
(`.ai/canvas-threading.md` §3).

**`setGroup(name, items)`:**

1. Deep-copy `items` into a fresh block, exactly as `emit_publish`
   (`gen_present.rs`) does for a scene — same code path, same guarantees.
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

The free itself happens on the **worker**, at the top of the next `present` — the same
place and the same reason the scene ring reclaims (§3 step 3). The graphics thread
never returns memory.

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
for each resolved group node, that group's **current `revision`**. Concretely: the
resolution pass writes `(slotIndex, revision)` into the published node, and the content
comparison sees a changed revision as a changed scene. So `setGroup` followed by
`present` with an unchanged list republishes and redraws; `present` twice with no
`setGroup` between still skips.

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
- [ ] Add the `Group` arms to `__canvas_headerFor` and `__canvas_tailFor` returning
      `__canvas_emptyHeader()` — a group has no geometry of its own.
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
- [ ] `setGroup` past `CANVAS_MAX_GROUPS` raises a named, trappable error.
- [ ] Tests: `tests/rt_canvas_present_deep_copy.rs` gains a group case — mutate the
      list the caller passed to `setGroup` and assert the installed group is unchanged.
      `removeGroup` of an absent name is a no-op.

Acceptance: the deep-copy and absent-name cases pass; nothing is freed yet, so this
phase can leak by construction — assert that it does, so Phase 5's gate has a
measurable "before".
Commit: —

### Phase 4 — Resolution, depth limit, frame skip, software rendering

- [ ] The resolution pass in `__canvas_present` per §4.4: names → slot indices,
      depth-first, depth > 64 raises.
- [ ] The depth error is a named trappable error with the §4.4 message, listed in
      `present`'s `errors:` and in `src/docs/spec/diagnostics/02_error-codes.md`.
- [ ] Fold each resolved group's `revision` into the published node so the content
      comparison sees a `setGroup` as a change.
- [ ] `__canvas_renderScene` walks resolved groups with an accumulated offset,
      offsetting bounds **before** the surface clamp (§4.5).
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
      executed on the **worker**, at the top of `present`, beside the scene ring's own
      reclaim.
- [ ] Extend `MFB_CANVAS_STATS` with `groups=` and `groupBytes=` so the leak is
      observable; this is the only window onto worker-owned state a test has
      (`.ai/canvas-threading.md` §11).
- [ ] Tests, as a race matrix in the style of `.ai/canvas-threading.md` §8 — add the
      rows to that document too:
      - `present([Group A])` → `removeGroup(A)` → graphics mid-frame: the in-flight
        frame completes normally.
      - the same, then a completed frame, then a `present`: the buffer is freed exactly
        once and `groups=` drops by one.
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
- [ ] `src/docs/spec/diagnostics/02_error-codes.md` — the two new errors.
- [ ] `scripts/man-run-examples.sh canvas --run` passes.
- [ ] `scripts/regen-ncodesum.sh`; prove the delta is this letter's.

Acceptance: `cargo test --no-fail-fast` green on both axes, `scripts/test-accept.sh`
green, `scripts/artifact-gate.sh all` 0 diffs, and `mfb man canvas setGroup` /
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

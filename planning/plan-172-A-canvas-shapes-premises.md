# plan-172-A: Canvas shape model — premises, populations and the letter map

Last updated: 2026-10-10
Overall Effort: huge (>3d) — the whole plan-172 canvas shape model, split A–M below
Effort: large (3h–1d)
Depends on: nothing

plan-172 implements `planning/new_canvas.md`: every shape carries its own `id`,
`position` and `transform`; `canvas::Point` gives way to `vector::Float2`;
`canvas::Rectangle` takes four corner radii and `canvas::RoundedRect` is removed; a
`canvas::Group` is a full affine; `canvas::AutoPaint` changes a shape's paint as the
pointer moves over it; `canvas::MouseEvent` reports `overID`; and a `canvas::Picture`
is clipped to a shape. It is a source-breaking rewrite of the canvas item surface.

Behavioral outcome of the whole plan: every type in `planning/new_canvas.md` above its
`## Future` heading exists exactly as written there, every renderer (software, Metal,
Vulkan) draws it, and every canvas program in the tree is ported to it.

The letters are in implementation order:

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | The premises the design rests on, each falsified or confirmed by a throwaway probe; nothing in `src/` changes | large |
| B | `canvas::Point` → `vector::Float2` everywhere | large |
| C | `transform` and `id` on every shape; `Paint.transform` and `Ellipse.angle` removed | large |
| D | `position` on `Circle`, `Ellipse`, `Arc`, `Text`; `Text.size` → `fontSize` | large |
| E | `Rectangle` with `position` / `bind` / `size` and four radii; `RoundedRect` removed; `Picture` moves to `position` / `size` | large |
| F | `position` and local points on `Line`, `Polygon`, `PolyLine` | large |
| G | `Group.position` + full affine `Group.transform`, software renderer, damage and hashing | large |
| H | The group affine on Metal and Vulkan | large |
| I | Hit-testing and `MouseEvent.overID` | large |
| J | The main thread publishes the pointer to the graphics state, on three platforms | large |
| K | The `Shape` / `AutoShape` / `DrawItem` unions and `AutoPaint` | large |
| L | `Picture` clipped to a shape; `PictureClip`, `PictureFit` | large |
| M | Closeout: `.ai/canvas-threading.md`, cross-platform proof, the full gates | large |

This letter builds nothing. It runs the six cheapest experiments that could reshape the
design, records what they found, and corrects the later letters where a premise fails.

References:

- `planning/new_canvas.md` — the type surface this plan implements. Source of truth for
  every record, field name and rule; where a letter and that file disagree, that file wins.
- `.ai/canvas-threading.md` — the three-thread model, §3 scene ring, §4 redraw triggers,
  §10 the group offset bound to both shader stages, §14 geometry cache and item hashes,
  "Growing `ITEM_BLOCK_SIZE` moves six things, and three are silent".
- `.ai/compiler.md` (Hard Completion Gate), `.ai/testing-gates.md` (canvas goldens are a
  bug hunt, never a re-baseline), `.ai/specifications.md`, `.ai/man-content.md`.
- `src/docs/spec/app/06_canvas.md` — the spec topic every letter that changes an
  observable contract updates in the same change.
- `src/codegen/builtins/canvas/mod.rs` — the registry and its five frozen-set tests.

## Prerequisites

These gate the whole of plan-172. Every other letter points here.

| Must be true | Command | Status |
|---|---|---|
| No plan has added a `DrawItem` variant beyond the eleven | `grep -c '"ParticleSystem"\|"Path"' src/codegen/builtins/canvas/mod.rs` → 0 | MET — measured 0 |
| plan-150 and plan-164 have not started | `git log --oneline -- src/codegen/builtins/canvas/helper_particles.rs \| wc -l` → 0 | MET — measured 0 |
| The canvas suites are green before any edit | `cargo test --test rt_canvas_golden --test rt_canvas_rasteriser --test rt_canvas_metal` → pass | MET — 22 / 86 (2 ignored) / 45 passed, 0 failed |
| `vector::Float2` exists as a value record of two `Float`s | `cargo run --bin mfb -- man vector types \| grep -c Float2` → ≥ 1 | MET — measured 1 |

If any row is NOT MET, plan-172 cannot start, full stop. Everything below is written
against the world where they hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- Each of the six premises in §4 is recorded in this file as CONFIRMED or FALSIFIED, with
  the probe program's path, the command and its observed output.
- For every FALSIFIED premise, the named alternative in §4 is adopted and the affected
  letter's design section is corrected in the same commit.
- The Open Decisions at the foot of this file are answered by the owner and recorded.

### Non-goals (explicit constraints)

- **Nothing under `src/`, `tests/`, `examples/`, `tools/` or `scripts/` changes in this
  letter.** Probes live in `/tmp/plan-172-A/`; a probe that needs a compiler change makes
  it in a `git worktree add --detach` checkout that is discarded.
- **No premise's failure stops the plan.** Each has a named alternative that reaches the
  same user-visible surface.
- **The type surface in `planning/new_canvas.md` is not renegotiated here.** A probe may
  change *how* a type is declared or carried, never what the user writes.

## 2. Current State

The canvas package is a registry of types (`src/codegen/builtins/canvas/mod.rs`) plus
functions that are either native emitters or MFBASIC source held in Rust string
constants. A scene is a `List OF DrawItem`, deep-copied by `present` and published
through the scene ring to the graphics thread (`.ai/canvas-threading.md` §3).

- **The item union is flat.** `DrawItem` lists eleven records in declaration order, and
  that order is each variant's tag (`mod.rs`, the `pkg.add_union` block; pinned by
  `draw_item_variant_set_is_frozen`). `RegistryUnion` has no `INCLUDES`
  (`src/codegen/registry/mod.rs:RegistryUnion` — `name`, `export`, `variants` only).
- **Placement is per-shape scalars.** `Rectangle` / `RoundedRect` / `Picture` share
  `rect_props` (`x`, `y`, `w`, `h`, `paint`); `Circle`, `Arc`, `Ellipse`, `Text` carry
  `x`, `y`; `Line` carries `x1`, `y1`, `x2`, `y2`; `Polygon.contours` and
  `PolyLine.points` are lists of `canvas::Point` in surface coordinates.
- **The transform lives on `Paint`.** `__canvas_paintHeader`
  (`helper_geometry.rs`) inverts it through `__canvas_invertTransform`
  (`helper_paint_defaults.rs`), the one place the inverse is computed and where the
  all-zero and no-area rules are implemented, and writes the inverse to geometry header
  slots 27–33. The native geometry builder declines any item whose transform is not
  all-zero (`func_geo_build.rs:emit_paint_supported`) and the MFBASIC builders take over.
- **A group is a translation.** `canvas::Group` is `dx`, `dy`, `name`. The draw entry is
  eight words — base, count, `dx`, `dy` as 16.16, blend mode, two residual words, one
  reserved (`helper_render.rs:__canvas_pushOneDraw`,
  `src/codegen/runtime/canvas/mod.rs:CANVAS_DRAW_ENTRY_WORDS`). Both GPU emitters bind
  16 bytes of it to the vertex and fragment stages
  (`src/target/macos_aarch64/app/metal.rs:emit_draw_list_pass`,
  `src/codegen/runtime/canvas/vulkan.rs:emit_draw_list_pass`).
- **Mouse events never touch the graphics state.** Each platform's main thread writes
  SGR-1006 text to the window input pipe (`src/target/macos_aarch64/app/mouse_view.rs`,
  `src/target/linux_gtk/mouse.rs`, `src/target/win_x86_64/app/mouse.rs`); the worker
  decodes it into a worker-local ring only while it is inside an `io::` read; and
  `canvas::pollMouse` only dequeues (`func_poll_mouse.rs`, `mouse_ring::emit_dequeue`).
  An event outside the surface is dropped, so nothing reports the pointer leaving.
- **There is no hit test.** The distance functions exist
  (`helper_draw.rs:__canvas_geoDistance`) but read the geometry cache, which today only
  the graphics thread fills.

### Measured populations

`$D` below is `src tests examples tools scripts benchmark`.

| What | Count | Command |
|---|---|---|
| Item constructors, all variants | 954 in 118 files | `grep -rhoE 'canvas::(Rectangle\|RoundedRect\|Circle\|Line\|Text\|Polygon\|PolyLine\|Arc\|Ellipse\|Picture\|Group)\[' $D \| wc -l` → 954; same with `-l` → 118 |
| `canvas::Rectangle[` | 250 in 63 files | `grep -rhoE 'canvas::Rectangle\[' $D \| wc -l` → 250 |
| `canvas::RoundedRect[` | 36 in 22 files | same pattern, `RoundedRect` → 36 |
| `canvas::Circle[` | 135 in 47 files | → 135 |
| `canvas::Line[` | 67 in 26 files | → 67 |
| `canvas::Text[` | 62 in 26 files | → 62 |
| `canvas::Polygon[` | 116 in 36 files | → 116 |
| `canvas::PolyLine[` | 47 in 16 files | → 47 |
| `canvas::Arc[` | 36 in 16 files | → 36 |
| `canvas::Ellipse[` | 33 in 19 files (28 set `angle`) | → 33; `grep -rhoE 'canvas::Ellipse\[[^]]*' $D \| grep -c 'angle *:='` → 28 |
| `canvas::Picture[` | 71 in 24 files | → 71 |
| `canvas::Group[` | 101 in 22 files (all set `dx`) | → 101; `… \| grep -c 'dx *:='` → 101 |
| `canvas::Point` mentions | 759 in 82 files | `grep -rhoE 'canvas::Point\b' $D \| wc -l` → 759 |
| `transform :=` assignments | 148 in 38 files | `grep -rhoE '\btransform *:=' $D \| wc -l` → 148 |
| `canvas::Transform[` constructors | 103 in 34 files | `grep -rhoE 'canvas::Transform\[' $D \| wc -l` → 103 |
| `cornerRadius` mentions | 69 in 29 files | `grep -rho cornerRadius $D \| wc -l` → 69 |
| `canvas::pollMouse` mentions | 23 in 13 files | `grep -rho 'canvas::pollMouse' $D \| wc -l` → 23 |
| Canvas registry records / unions / enums | 21 / 1 / 10 | `grep -c 'pkg.add_record(' src/codegen/builtins/canvas/mod.rs` → 21; `add_union(` → 1; `add_enum(` → 10 |
| Scene files / PNG goldens | 38 / 8 | `ls tests/canvas/scenes \| wc -l` → 38; `ls tests/golden/canvas/*.png \| wc -l` → 8 |
| Item block size | 368 bytes | `grep -n 'const ITEM_BLOCK_SIZE' src/codegen/runtime/canvas/mod.rs` → `= 368` |
| Draw entry width | 8 words | `grep -n 'const CANVAS_DRAW_ENTRY_WORDS' src/codegen/runtime/canvas/mod.rs` → `= 8` |
| Geometry header | 67 floats | `grep -n '__CANVAS_GEO_HEADER AS Integer' src/codegen/builtins/canvas/helper_geometry.rs` → `= 67` |
| Highest `7705xxxx` error code | 77050027 | `grep -rhoE '7705[0-9]{4}' src build.rs \| sort -u \| tail -1` → 77050027 |

### Verified properties

- **A registry record may hold a union-typed, recursive field.** VERIFIED by reading
  `src/codegen/builtins/regex/mod.rs`: `__regex_Repeat.child AS __regex_Node` where
  `__regex_Node` is a union that lists `__regex_Repeat`.
- **Field offsets and variant tags are looked up by name in the native emitters.**
  VERIFIED by reading `func_geo_build.rs` (`variant()`, `record_fields`) and
  `func_carried_hashes.rs` (`variant_tag`): a renamed or removed field breaks the build;
  a reordered variant breaks no emitter. The native hash folds the tag first
  (`func_item_hash.rs:emit_item_hash`), so hashes change value, which only moves
  in-process cache keys.
- **Three layouts are hand-built and do not follow the type model.** VERIFIED:
  `func_poll_mouse.rs` (`MOUSE_EVENT_BLOCK_BYTES = 64`, the inlined `Point` at offset
  48); `__canvas_hashItem`'s literal start tags (`helper_geometry.rs`); the test-only
  `FIELD_KIND_TABLE` (`src/codegen/collection/assign/self_update.rs`) with its generator
  `tests/runtime/inplace_self_update/field_kinds_gen.py`.
- **A timed condition wait already exists on all three operating systems.** VERIFIED by
  reading `src/codegen/runtime/canvas/mod.rs:emit_await_pickup`
  (`pthread_cond_timedwait` on `FRAME_COND`) and its Windows arm in
  `src/codegen/runtime/thread/runtime_helpers.rs`.
- **The artifact gate does not cover canvas rendering.** VERIFIED in
  `.ai/testing-gates.md`: a green `scripts/artifact-gate.sh` says nothing about canvas
  pixels. The instruments are the `rt_canvas_*` suites.
- **UNVERIFIED — the six premises of §4.** They are this letter's tasks.

## 3. Design Overview

The plan has two halves with different gates.

**Letters B–F reshape the records and change no pixel.** Each ports the scenes it
touches faithfully, so the eight PNG goldens and every pixel assertion must hold
unchanged. A golden that moves in B–F is a bug in the port or the builder, to be
localized on one scene.

**Letters G–L add behaviour.** Their gates are new runtime tests and new goldens. The
existing goldens must still hold, because each new capability has an inert zero value
(identity group transform, no `AutoPaint`, a plain-rectangle `Picture`).

**Byte-identity is not this plan's gate.** Codegen for every canvas program changes.
The `tests/syntax/app/app-mouse-surface` goldens (`.ir`, `.nir`, `.nplan`, four
`.ncodesum`) and `tests/syntax/resources/loop-body-resource-reference-valid` are
EXPECTED to diff in every letter that touches the registry; they are regenerated per
`.ai/testing-gates.md`, and a diff there is the plan working.

**Where design uncertainty concentrates:** the six premises below. They are scheduled
first, here, as throwaway probes.

**Where correctness risk concentrates:** letter H (a per-draw matrix through two
hand-written GPU emitters and two shaders) and letter K (a second writer of
render-affecting state, on a thread that cannot raise). Both sit behind the software
oracle that the preceding letter lands.

### The identity rule that keeps B–F pixel-neutral

A shape's effective placement is `T(position) ∘ transform`. Building it that way for
every item would send every item with a non-zero `position` down the transformed path:
the native builder declines it, the rectangular clip fast path is lost, and
antialiasing is computed through an inverse map rather than directly.

So the builders split on the transform:

- **All-zero `transform`:** `position` is added to the geometry's own coordinates at
  build time. The geometry record is byte-for-byte what today's builder writes for the
  same surface coordinates, and `hasTransform` stays 0.
- **Any other `transform`:** the geometry is built in local coordinates and the effective
  affine is the shape's linear part with translation `position + (tx, ty)`.

`Paint.clip` rings and `Paint.gradient` points follow the same split.

### Rejected alternatives

- **One letter for the whole record reshaping.** Rejected: 954 constructor sites in one
  sitting, with no golden able to say which family's port went wrong.
- **Keeping `Paint.transform` as a deprecated alias through the migration.** Rejected:
  two transforms on one item need a composition order nobody asked for, and the alias
  would have to be deleted by a later letter anyway.
- **An exact distance function for per-corner and elliptical rectangle corners.**
  Rejected for letter E in favour of flattening such a rectangle to a polygon ring: an
  exact field needs a new geometry kind on three renderers, and a dashed stroke would
  need the arc length of an ellipse quadrant, which has no closed form. See 172-E.
- **A picture as a paint source on any shape geometry.** Rejected for letter L in favour
  of clipping the picture rectangle to the shape's outline: the paint-source form needs
  new header slots that collide with the aux and cap slots closed shapes already use,
  and a wider item block on both GPU backends. See 172-L.

## 4. The six premises

Each is a probe program under `/tmp/plan-172-A/`, a command, and a named alternative.

| # | Premise | Probe | If FALSIFIED |
|---|---|---|---|
| P1 | A builtin union can be declared with `INCLUDES`, so `Shape`, `AutoShape` and `DrawItem` nest and one record is a member of all three | In a detached worktree add `includes: Vec<&'static str>` to `RegistryUnion`, emit `UNION X INCLUDES Y` from `render()`, declare the three unions over the existing records, build a canvas program that `MATCH`es a `DrawItem` and a `Shape` | Declare three flat unions in the registry with the same member sets. The user-visible types are identical; only `mfb man canvas types` shows lists rather than `INCLUDES` |
| P2 | A `DrawItem` variant with a union-typed field (`AutoPaint.shape AS Shape`) survives `present`'s deep copy, `canvas::setGroup`'s flat copy, the native item hash and scope drop | In the same worktree add a one-field record `Probe[shape AS Shape]` as a variant; present it, install it in a group, drop it, 1,000 times under `--debug` arena checks | The copy path is fixed first, as a bug-NN document with the probe as its reproducer, and that fix becomes a prerequisite of 172-K. The type surface does not change |
| P3 | `canvas` can import `vector` and name `vector.Float2` in a record prop, including as the inlined `MouseEvent.position` | In the worktree add `"vector"` to `pkg.add_imports`, retype `Point`-typed props of `Gradient` to `vector.Float2`, build and run a gradient scene; record the `.ncode` size of an empty canvas program before and after | Wire a late injection pass for `vector` on the `color::augmented_project` precedent (`src/ir/lower.rs`, `src/resolver/mod.rs`) as the first phase of 172-B, and record the byte cost |
| P4 | The worker can build geometry and evaluate a distance for an item it holds, using its own thread-local geometry cache | A canvas program whose `main` calls an internal probe function wrapping `__canvas_geometryFor` + `__canvas_geoDistance` for a `Circle`, and prints the distance at three points | The hit test builds the geometry record into a caller-owned `List OF Float` and evaluates distance against that list; 172-I gains a phase to factor `__canvas_geoAt` readers over a list argument |
| P5 | `pollMouse` can become an MFBASIC wrapper over a native raw poll and return a record holding a `String` | In the worktree rename the native to `pollMouseRaw` (`internal_only`), add an MFBASIC `pollMouse` returning `raw WITH { overID := "x" }`; run `tests/syntax/app/app-mouse-surface` and a `MFB_MOUSE_INJECT` run | The native emitter writes the string slot itself; 172-I gains a task to read the String representation from `.ai/codegen-invariants.md` and build it in `func_poll_mouse.rs` |
| P6 | A record constructor must name every field, so the new shapes need constructor functions to be usable | `canvas::Circle[x := 1.0, y := 1.0, radius := 2.0]` (no `paint`) through `cargo run -- build`; record the diagnostic | If a field may be omitted, drop the constructor functions from D, E and F and Open Decision 1 |

### Results (measured 2026-10-10)

Every probe program, edit script and captured output is under `/tmp/plan-172-A/`; the
compiler changes the probes needed are one diff, `/tmp/plan-172-A/probe.diff` (17 files),
taken from the discarded worktree `/tmp/plan-172-A/wt` at `29e56cf34`. Programs were
built with `/tmp/plan-172-A/mk.sh <mfb> <name> <source> [--debug]` and run headless
(`MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1`) on macOS aarch64.

| # | Verdict | Probe | Command and observed output |
|---|---|---|---|
| P1 | **CONFIRMED** | `p1.mfb` | `RegistryUnion.includes` rendered as `UNION X INCLUDES Y` parses and nests. `p1` printed `item:Circle`, `shape:Circle`, `auto:Circle`, `item:Probe`: one `Circle` value is accepted as a `DrawItem`, a `Shape` and an `AutoShape`, and each union `MATCH`es it. Tags: an included union's members come first, in its own order, then the local members (`src/binary_repr/writer.rs:concrete_union_variants`), so the three unions agree on every shared tag — `Shape` is a prefix of `AutoShape` is a prefix of `DrawItem`. With `new_canvas.md`'s declarations: Rectangle 0, Line 1, Polygon 2, Circle 3, Arc 4, Text 5, Ellipse 6, PolyLine 7, AutoPaint 8, Picture 9, Group 10. Adding a variant makes four companion `MATCH`es non-exhaustive (`TYPE_MATCH_NOT_EXHAUSTIVE`, all in `helper_geometry.rs`); the native emitters built unchanged |
| P2 | **CONFIRMED** | `p2.mfb`, `run_p2.sh` | `sh run_p2.sh 1000` → `sum=20000`, exit 0. A `Probe[shape AS Shape]` variant presented, installed in a group and dropped per round. Worker arena `live_bytes` 31568 at 10 rounds and 31568 at 1,000; graphics arena 28208 and 28208; `double_free_skips 0` on both. The nested union read back through two `MATCH`es each round |
| P3 | **FALSIFIED**, alternative adopted and proven | `empty.mfb`, `empty_vec.mfb`, `p3.mfb` | With only `"vector"` in `pkg.add_imports`, a canvas program that does not itself `IMPORT vector` fails: six `TYPE_UNKNOWN_VALUE` in the companion (`g.startPoint.x` and its five siblings in `helper_geometry.rs`), because the generic injection pass never injected `vector`. With the late pass wired on the `color` precedent (`vector::augmented_project`, the generic-pass skip, the two AST call sites and the HIR one) the same program builds and `p3` renders a red→blue ramp through the **native** geometry builder, which read `vector.Float2`'s `x` / `y` by name unchanged (pixels at x = 45 / 200 / 355: `[253,0,35]`, `[187,0,188]`, `[32,0,253]`). Byte cost, `ls -l` of the executable: 2,666,000 before → 2,996,236 after, **+330,236 bytes on every canvas program**; a program that already imports `vector` is the same 2,996,236, so the cost is `vector`'s whole companion and nothing else |
| P4 | **CONFIRMED** | `p4.mfb`, `run_p4.sh` | `sh run_p4.sh` → `centre=-20.00`, `edge=0.00`, `outside=30.00`, `other=-5.00`, `again=-20.00`, exit 0. By hand for a circle at (40, 40) radius 20: −20, 0, and √(30² + 40²) − 20 = 30. The worker called `__canvas_geometryFor` + `__canvas_geoDistance` with no `present`, so its own thread-local cache held the records; a second item and a re-query of the first both answered correctly |
| P5 | **CONFIRMED** | `p5.mfb`, `run_p5.sh` | `sh run_p5.sh` (one injected `ESC[<0;10;20M`) → `event at 9.00,19.00 overID=[x]`, then `idle overID=[x]` twice, exit 0, `double_free_skips 0`. The native poll returned an internal six-field record and the MFBASIC wrapper built the seven-field `MouseEvent` by name; `position` was a `vector.Float2` inlined exactly as `Point` was (the native block stayed 64 bytes) |
| P6 | **CONFIRMED** | `p6.mfb` | `mk.sh … p6 p6.mfb` → `error[2-203-0024 TYPE_CONSTRUCTOR_ARITY_MISMATCH]: constructor has the wrong number of arguments — Constructor `canvas.Circle` has 3 argument(s), expected 4.` A constructor names every field, so the shape constructor functions stay in D, E and F |

Four things the probes found that no premise asked about. Each is written into the
letter it lands in:

1. **A program that reads a `vector::Float2`'s fields must `IMPORT vector` itself.**
   `e.position.x` on a `canvas::MouseEvent` without it is `TYPE_UNKNOWN_VALUE`: "Field
   `x` belongs to `vector::Float2`, whose fields are not visible in this file. Imports
   are not transitive…". So `tests/syntax/app/app-mouse-surface` gains `IMPORT vector`
   in 172-B, as does every program that reads or builds a coordinate pair.
2. **`canvas.pollMouse` is spelled in six per-target tables** besides its registration
   (`src/target/{macos_aarch64,win_x86_64,linux_common}/{mod,plan}.rs`), and in
   `src/codegen/registry/mod.rs`'s record-returning list. The rename to `pollMouseRaw`
   in 172-I moves all of them; missing one is `native backend does not support runtime
   call 'canvas.pollMouseRaw'`.
3. **Mouse state is reserved by symbol suffix.** `module_uses_mouse`
   (`src/target/shared/plan/symbols.rs`) and two sites in
   `src/codegen/engine/builder/mod.rs` test `ends_with("_pollMouse")`. A raw poll named
   `pollMouseRaw` does not match, and the build fails with `native code plan emits
   '_mfb_rt_canvas_canvas_pollMouseRaw' without reserving mouse state`.
4. **An MFBASIC `pollMouse` is compiled into every canvas program, used or not.** `p4`
   never names the mouse and still emitted the raw poll. So once the three suffix tests
   accept the raw name, every canvas program reserves mouse state unless the wrapper is
   a gated helper. 172-I registers it gated on use.

## Compatibility / Format Impact

This letter changes nothing observable. The plan as a whole changes:

- **Source-breaking, the whole canvas item surface:** every record in
  `planning/new_canvas.md`. No compatibility shim is kept.
- **Removed:** `canvas::Point`, `canvas::RoundedRect`, `Paint.transform`,
  `Ellipse.angle`, `Group.dx` / `dy`, `Rectangle` / `Picture` `x` / `y` / `w` / `h`,
  `Text.size`.
- **Internal layouts that move:** the draw entry (8 → 16 words, letter G), the per-draw
  GPU payload (16 → 32 bytes, letter H), the graphics-state block (letter J), the
  `MouseEvent` block (letters B and I).
- **Unchanged:** `ITEM_BLOCK_SIZE` (368), the geometry header (67 floats), the scene
  ring, the group table, `Paint`'s other thirteen fields, `canvas::Transform`'s six
  fields and its all-zero-is-identity rule, every function not named in a letter.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Language and registry premises (P1, P2, P6)

The three that decide how the types are declared. Cheapest, and K cannot be designed
without them.

- [x] Create a detached worktree: `git worktree add --detach /tmp/plan-172-A/wt`.
      → `HEAD is now at 29e56cf34`.
- [x] P6: build the omitted-field probe; record the diagnostic text and code in §4.
      → `2-203-0024 TYPE_CONSTRUCTOR_ARITY_MISMATCH`, CONFIRMED.
- [x] P1: make the `RegistryUnion` change in the worktree and build the three-union
      probe. Record whether one record may be a member of three builtin unions and what
      tags each union assigns.
      → CONFIRMED; `p1` printed all four names; tags are prefix-consistent (§4 Results).
- [x] P2: add the `Probe` variant and run the present / setGroup / drop loop under
      `--debug`. Record leaks, double frees or a clean run.
      → CONFIRMED; `sh /tmp/plan-172-A/run_p2.sh 1000` → clean, `live_bytes` flat.
- [x] Record each result in §4 as CONFIRMED or FALSIFIED with the command and output.
      → `grep -c 'CONFIRMED\|FALSIFIED' planning/plan-172-A-canvas-shapes-premises.md` ≥ 3.

Acceptance: §4 rows P1, P2 and P6 each carry a verdict, a probe path and observed
output; a FALSIFIED row's alternative is written into 172-K (P1, P2) or 172-D/E/F (P6).
  Check: `grep -c 'CONFIRMED\|FALSIFIED' planning/plan-172-A-canvas-shapes-premises.md`
  → ≥ 3 (est. 1 min to check; the probes themselves est. 2 h).
Commit: —

### Phase 2 — Package and thread premises (P3, P4, P5)

- [x] P3: import `vector` into `canvas` in the worktree; build and run a gradient scene
      and a `MouseEvent` read. Record the empty-program `.ncode` size before and after
      with `ls -l`, and whether a late injection pass was needed.
      → FALSIFIED; the late pass is needed and works; 2,666,000 → 2,996,236 bytes.
- [x] P4: build the worker-side distance probe; compare its three distances with the
      values computed by hand for the circle.
      → CONFIRMED; −20 / 0 / 30 observed, −20 / 0 / 30 by hand.
- [x] P5: build the `pollMouse` wrapper probe; run it under `MFB_MOUSE_INJECT` with one
      injected press and print `overID`.
      → CONFIRMED; `event at 9.00,19.00 overID=[x]`.
- [x] Record each result in §4, and correct 172-B (P3) and 172-I (P4, P5) where a
      premise failed.
      → 172-B Phase 1 and §3 corrected for P3 and finding 1; 172-I Phase 3 corrected for
        findings 2–4 (P4 and P5 held, so its design stands).
- [x] Remove the worktree: `git worktree remove --force /tmp/plan-172-A/wt`.
      → `git worktree list | grep -c plan-172-A` → 0. `--force` because the probe edits
        were uncommitted by design; they survive as `/tmp/plan-172-A/probe.diff`.

Acceptance: §4 rows P3, P4 and P5 each carry a verdict, a probe path and observed
output, and the main checkout is untouched.
  Check: `git status --porcelain src tests examples tools scripts` → empty (est. 1 min;
  probes est. 3 h).
Commit: —

### Phase 3 — Decisions recorded

- [ ] Put Open Decisions 1–5 below to the owner and record each answer in place.
- [ ] Apply each answer to the letter it names.

Acceptance: no Open Decision in this file is unanswered.
  Check: `grep -c 'ANSWER:' planning/plan-172-A-canvas-shapes-premises.md` → 5
  (est. 1 min).
Commit: —

## Validation Plan

- Tests: none added; this letter lands a record of measurements only.
- Coverage check: not applicable — no code changes.
- Runtime proof: the six probe programs and their recorded output in §4.
- Doc sync: none in this letter. Each later letter updates
  `src/docs/spec/app/06_canvas.md` and the registry descriptions in the same change.
- Final gate: none here. The plan's full gates run once, in 172-M.

## Open Decisions

1. **Constructor functions for the shapes.** A `Rectangle` now has ten fields and
   building it by name lists all ten. Recommended: one function per shape taking the
   fields with no useful zero — `canvas::rectangle(position, size, paint)`,
   `canvas::circle(position, radius, paint)` and so on — leaving `id` empty, `bind`
   `TopLeft`, radii `0.0` and `transform` all-zero, refined with `WITH`. This is the
   `canvas::fill` / `canvas::stroke` precedent for `Paint`. The alternative is no
   helpers, and every site lists every field. (172-D, E, F; depends on P6.)
2. **Non-uniform and elliptical rectangle corners are flattened to a polygon.**
   Recommended (172-E). The outline is within 0.05 local pixels of the true curve, and a
   group or shape scale magnifies that error. The alternative is an exact field on three
   renderers with no closed-form dash length.
3. **`paint.clip` on a `Picture` whose shape is not a plain rectangle.** The picture's
   one clip slot is taken by the shape outline. Recommended: `paint.clip` then applies to
   the shape's outline stroke only, and `present` raises a new error if such a picture
   also sets `paint.clip`, rather than silently ignoring it. The alternative is polygon
   intersection of two ring sets, which this plan does not build. (172-L.)
4. **Ordering against plan-164 (`canvas::Path`) and plan-162 (text).** plan-164 appends
   a `DrawItem` variant and uses `canvas::Point`; it collides with this plan exactly as
   plan-150 does. Recommended: plan-164 takes plan-172 as a prerequisite, as plan-150 now
   does. plan-162 changes text shaping, not the item records, and can run either side.
5. **`MouseEvent.overID` needs the worker's own geometry.** If P4 is confirmed the worker
   keeps a second geometry cache for the scene it last presented, built lazily on the
   first poll after a `present`. Recommended: accept that memory, and build it only when
   the mouse is enabled. The alternative is to resolve `overID` on the graphics thread
   and report the id as of the last rendered frame. (172-I.)

## Corrections

- **Prerequisite row 4's command did not run as written.** `cargo run -- man vector
  types` fails with "`cargo run` could not determine which binary to run … available
  binaries: mfb, mfb-repo". Corrected to `cargo run --bin mfb -- …`, which prints 1.
- **P3 is FALSIFIED.** Importing `vector` is not enough; see §4 Results. The named
  alternative (a late injection pass) was built in the probe worktree and works, so
  172-B's first Phase 1 task is unconditional. The measured cost is +330,236 bytes on
  every canvas program.
- **P3's probe was run with the late pass wired**, which the row did not ask for, so the
  alternative is proven rather than assumed.
- **P5's row said to "run `tests/syntax/app/app-mouse-surface`".** That fixture reads
  `e.position.x` and does not `IMPORT vector`, so under the probe compiler it fails for
  the reason in finding 1, not for anything P5 is about. `p5.mfb` (the same reads, with
  the import) was run instead.
- **P2's row named `canvas::setGroup`'s flat copy and the native item hash.** Both ran
  in `p2` (the `Probe` is installed in a group and presented each round). The `Probe`
  variant hashed through a companion `__canvas_hashItem` arm the probe added, not
  through a native arm; 172-K writes the real `AutoPaint` hash.
- **`/tmp/plan-172-A/wt` was removed with `--force`.** The task's bare
  `git worktree remove` refuses a worktree with uncommitted changes, and the probe's
  changes were never meant to be committed.

## Summary

Nothing here ships to a user. The value is that six unproven claims — three about the
language and registry, three about packages and threads — are settled before any of
the 954 constructor sites is touched, and that each has a way forward whichever way it
falls. The real engineering risk of plan-172 is in letters H and K; this letter exists
so that neither is designed on a guess.

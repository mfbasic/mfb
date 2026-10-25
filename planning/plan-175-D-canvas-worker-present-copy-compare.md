# plan-175-D: The worker's present compares before it copies, and does no per-item work for a scene it will not publish

Last updated: 2026-10-25
Effort: medium (1h–2h)
Depends on: nothing. plan-175-A's Phase 2 depends on THIS letter (plan-175-A decision 3).

plan-175-A §8.1 names the worker's deep copy and content compare as the largest cost on
the Mac at 100,000 dots, and the owner ruled (plan-175-A decision 3, 2026-10-12) that it
comes before plan-175-A's Phase 2. No letter covered it; this one does (plan-175-A
Correction 3).

The single outcome: `canvas::present` and `canvas::presentLayers` of a scene equal to the
installed one copy nothing, free nothing, carry no hash and walk no item in MFBASIC; and
a scene that names no `Group` and no `ParticleSystem` is never walked item by item in
MFBASIC on the worker. The ownership contract, the frame skip's answer and every pixel
are unchanged.

References:

- `planning/plan-175-A-canvas-geometry-identity-split.md` — §2.3 (the `same` rows,
  finding 3), §8.1, decision 3.
- `.ai/canvas-threading.md` §3 (the ordering of `canvas::present`), §3.1 (the content
  compare).
- `tools/canvas-bench/README.md` — `dots N`, now with `workerPresentUs`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| The dots bench reports the worker's time per present | `tools/canvas-bench/bench.sh target/release/mfb dots 1000 2 \| grep -o 'workerPresentUs=[0-9]*'` → a number | MET 2026-10-25 (this letter's first commit adds it) |

## 1. Goal

- `canvas::publishScene` / `canvas::publishLayers` compare the CALLER's list against the
  installed one before copying it. An unchanged scene allocates nothing and frees
  nothing (today it copies the whole scene, compares, and frees the copy — whose free
  scrubs every byte, `mfb spec memory arenas`).
- `canvas::carriedHashes` runs only when the present will publish hashes (the scene
  changed, or a group it names did). Today it runs on every present.
- `__canvas_groupSignature` — an MFBASIC `FOR EACH` + `MATCH` over every item of the
  scene — runs only when the scene's top level holds a `Group` or a `ParticleSystem`,
  which are the only items it reads. A native tag scan (`canvas::sceneNamesGroups`)
  decides.

### Non-goals

- No change to what is copied when a scene IS published, to the retirement ring, to the
  hashes, or to what the frame skip answers.
- No change to the graphics thread.

## 2. Current State (measured 2026-10-25, Apple M2 Max, load 9 to 11)

`tools/canvas-bench/bench.sh target/release/mfb dots 100000 4 [--same]`:
`workerPresentUs=37320` (paint, alternating scenes) and `32058` (same scene every time).

`sample` of a `--debug` build of each scene (`/tmp/p175-prof`, 4 s), worker self time:

| Function | paint (samples) | same (samples) |
|---|---:|---:|
| `canvas::carriedHashes` | 615 | 281 |
| `canvas::publishScene` (copy + compare + free) | 511 | 693 |
| `canvas::sceneHashes` | 372 | — |
| `canvas::sceneIdBits` | 109 | — |
| `__canvas_groupSignature` | 11 | 321 |

So in the `same` scene every present pays a copy of the scene, a full compare, a free
that scrubs the copy, a carried-hash pass whose result is dropped, and an MFBASIC walk of
all 100,000 items for a group signature of a scene with no group.

`__canvas_present` (`func_present.rs`), in order: the group drain; `carried =
canvas::carriedHashes(items)`; `installed = canvas::publishScene(items)`; `sig =
__canvas_groupSignature(items, 0)`; `moved`; then, if `installed OR moved`, the id bits,
`publishHashes(__canvas_hashScene(items, carried))` and the redraw signal.
`__canvas_presentLayers` (`func_present_layers.rs`) is the same without the carry.

## 3. Design

- `gen_present.rs:emit_publish`: the content compare (`emit_values_content_equal_branch`)
  moves ahead of `copy_flat_block` and compares the incoming list with the installed
  one. The comparison is by type, so the caller's headroom and the installed copy's
  tightness do not matter (§3.1 of the threading notes). The skip path frees nothing,
  because nothing was allocated; the publish path copies and publishes as today.
- `canvas::sceneUnchanged(items) AS Boolean` (internal, native, the mode gate first):
  the same compare without a publish, so `__canvas_present` can decide to carry BEFORE
  the scene is replaced. `canvas::installScene(items)` (internal, native) is the publish
  path of `emit_publish` without its compare, so the scene is compared once per present.
  `canvas::publishScene` keeps its contract (compare, then copy and publish) for
  `presentLayers` and every test that names it.
- `__canvas_present`: `unchanged = sceneUnchanged(items)`; if not, carry, then
  `installScene`. The group signature is taken only when
  `canvas::sceneNamesGroups(items)`; otherwise it is `[]`, which is exactly what the walk
  returns for such a scene. If only `moved`, the carry is taken then (against the scene
  still installed, as today).
- `__canvas_presentLayers`: the per-layer signature walk is gated the same way, layer by
  layer.

Why it is safe: the walk is skipped only for a list whose top level holds neither of the
two tags its `MATCH` acts on; every other arm is `sig = sig`. A `Group` reached through a
`ParticleSystem`'s template or an `AutoPaint`'s shape is not walked today either. The
compare is the same emitter over the same two lists; only the left operand changes from
the copy to the list the copy would have been made from, which holds the same content.

## Phases

### Phase 1 — Compare first, carry and walk only when they are used

- [x] `tools/canvas-bench`: `workerPresentUs=` on the `dots` row (`gen/dots.sh` sums the
      time inside `canvas::present`); README. (`bench.sh target/release/mfb dots 100000 4`
      prints `workerPresentUs=37320` at the base, e631f9144.)
- [x] `gen_present.rs:emit_publish`: compare before the copy; the skip path allocates and
      frees nothing. (The skip exit has no `emit_free_block`; the copy is after the
      `publish` label.)
- [x] ~~`canvas::installScene`~~ `canvas::sceneUnchanged`, `canvas::sceneNamesGroups`
      (native, internal-only), and `publishScene(items, compare)` (Correction 1);
      `__canvas_present` and `__canvas_presentLayers` use them.
- [x] Tests: an unchanged present allocates nothing —
      `rt_canvas_present_leak.rs:an_unchanged_re_present_copies_nothing`, by
      `arena.0.alloc_bytes` (Correction 2): equal at 120 and 240 presents. The same probe
      (200 rectangles) at the base allocates 148,992 bytes a present
      (`(45476848 - 15678448) / 200`), after 0 (`arena.0.alloc_calls 1136` at 100 and at
      300 presents). A `setGroup` under an unchanged scene still redraws, and a cycle
      and a zero-lifetime system still raise from both calls: already held by
      `rt_canvas_rasteriser` `groups::a_group_replaced_between_two_identical_presents_
      redraws_with_the_new_contents`, `groups::present_layers_sees_a_group_replaced_
      under_the_same_name`, `groups::a_group_cycle_and_an_over_deep_chain_both_raise`,
      the layered cycle row beside it, and `rt_canvas_particles`
      `lifecycle_and_model::a_lifetime_of_zero_raises_from_present` (present and
      presentLayers) — all pass (Correction 3).
- [x] `.ai/canvas-threading.md` §3 (the ordering) and the spec's present text
      (`06_canvas.md`, "A published scene points at nothing caller-owned").
- [x] Found doing this: `rt_canvas_graphics_thread`'s two group frame-skip tests were
      still `#[ignore]`d for a plan-116-G phase that landed, and one could never pass
      (no `MFB_CANVAS_DAMAGE`). Un-ignored and fixed in 87a2ae005 (failed at the base
      2f281c5e7 with `0,0,900,640` twice; passes now).

Acceptance: an unchanged present allocates nothing; the `same` row's worker time falls;
no test and no pixel changes.
  Check: `tools/canvas-bench/bench.sh target/release/mfb dots 100000 4 --same` →
  `workerPresentUs` below the 32,058 measured in §2 (same machine, load recorded).
  Check: `tools/canvas-bench/bench.sh target/release/mfb dots 100000 4` →
  `workerPresentUs` recorded beside §2's 37,320.
  Check: `cargo test --release --test rt_canvas_present_deep_copy --test rt_canvas_present_leak --test rt_canvas_group_ownership
  --test rt_canvas_damage --test rt_canvas_geo_native` → pass.

**MET 2026-10-25** (load 9 to 14, as §2's):
- `dots 100000 4 --same` → `workerPresentUs=3824` (32,058 at the base): 8.4 times less.
- `dots 100000 4` → `workerPresentUs=25250` (37,320 at the base), `pps=23 fps=24`
  (14 / 14 at the base, same machine and load band).
- `cargo test --release --test rt_canvas_present_leak --test rt_canvas_present_deep_copy
  --test rt_canvas_group_ownership --test rt_canvas_damage --test rt_canvas_geo_native
  --no-fail-fast` → 7, 9, 19, 12, 4 passed; `rt_canvas_rasteriser -- groups::` 19
  passed; `rt_canvas_particles -- a_lifetime_of_zero_raises_from_present` 1 passed;
  `rt_canvas_graphics_thread` 20 passed (the two un-ignored included).
Commit: —

## Corrections

1. 2026-10-25: no `canvas::installScene`. `canvas::publishScene` takes a second
   argument, `compare`, instead (FALSE: the caller has compared). It keeps ONE flat
   publisher, so the shape tests that pin it (`rt_canvas_present_deep_copy`: the mode
   gate before the allocation, the publish order, the skip writing nothing) still read
   the function that publishes, and the scene is compared once a present.
2. 2026-10-25: the test measures `arena.0.alloc_bytes`, not `perf.mfb_alloc.count`:
   `alloc_bytes` counts what was allocated whether or not it was freed, per arena, so
   the worker's arena is read alone. The first cut of `__canvas_present` still made 3
   small allocations a present (144 bytes: two `[]` literals and the signature
   global's copy); it was restructured so an unchanged present builds no list, and the
   test asserts exact equality.
3. 2026-10-25: the `setGroup`-redraw and raise behaviours already had tests in
   `rt_canvas_rasteriser` and `rt_canvas_particles` (named in the task); no new ones
   were written for them. The publish of a changed scene moved after the signature walk
   (into `__canvas_presentInstall`), so a present that raises from that walk no longer
   leaves its scene installed without hashes.

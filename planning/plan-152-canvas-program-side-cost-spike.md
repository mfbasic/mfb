# plan-152: Spike — where a canvas program's own per-frame cost goes, and what can remove it

Last updated: 2026-09-24
Overall Effort: large (3h–1d) for the spike; the fixes it recommends are planned separately
Depends on: bug-686 (the renderer side, so the program side is the measured limit)

## Why

bug-686 made the renderer fast enough that the **program** is now the limit. With a release
build and `MFB_CANVAS_SYNC=1`, 10,000 moving quads run at 24.8 fps. Of the ~40 ms frame,
**~25 ms is the program building its own `List OF canvas::DrawItem`**: 2.5 µs per item
(`workerBuildMs=` from `bash /tmp/bug686-spike/run_release_fps.sh <mfb> tag 10000 4 1 5`).
Only then does `canvas::present` (0.35–0.56 µs per item) or the graphics thread (~0.3 µs
per item resolve plus ~5 ms render) get a turn.

The per-item program cost therefore sets the ceiling for any MFBASIC canvas program:
~6,600 items at 60 fps at 2.5 µs, before the program has simulated anything. Nobody
knows yet whether those 2.5 µs are:

- **(a) codegen waste**, removable without changing what the language means (an avoidable
  copy, a missed in-place path, an allocation the value could live without);
- **(b) a consequence of the value semantics**, needing a language-level feature such as
  moves into constructors, reuse of the previous frame's list, or presizing; or
- **(c) inherent**: allocation of a variable-width value per item, which only a different
  API shape avoids.

The spike's job is to put every one of the 2.5 µs into one of those three bins, with a
measurement behind each, and to say which bin is worth a plan. It changes no compiler code
that lands. Probes live in `/tmp` or in this doc; a reusable instrument goes under
`tools/canvas-bench/` (AGENTS.md).

Goal for the recommendation, as settled for bug-686: **~10,000 moving items at 60 fps,
with the API a beginner already uses** (`present` a `List OF DrawItem`). 20,000 is the
stretch. The budget is 16.7 ms for the program's build plus its simulation, so the build
must fall to **≤1 µs per item** for 10k, and ≤0.5 µs for 20k.

## The workload under test

The stress program (`/tmp/bug686-spike/gen_stress.sh`, also `tools/canvas-bench/gen/stress.sh`).
Per item, per frame:

```
FUNC ring(cx, cy, r) AS List OF canvas::Point           ' 4 appends, 8 trig calls
  MUT points AS List OF canvas::Point = []
  WHILE i < 4 ... points = collections::append(points, canvas::Point[...]) ...
FUNC bulk(shift) AS List OF canvas::DrawItem
  LET it AS canvas::DrawItem = canvas::Polygon[points := ring(...), paint := canvas::fill(...)]
  items = collections::append(items, it)
```

Then the real one: `examples/wind` `frameScene`. It does these every frame:

- `mapItems`: rebuilds ~coast polygons from `List OF Ring` for the current view.
- `trailItems`: one `canvas::Line` per particle segment, with a `canvas::stroke` paint
  built per segment.
- A `FOR EACH item IN trailItems(v) … items = collections::append(items, item)` merge.
- Its flow simulation.

Its worker previously measured ~22 ms per frame in release (bug-686 G3 notes).

## Phase A — the attribution table (measure; change nothing)

Build every probe **release** (`mfb build -app`, no `--debug`: debug inflates allocation
code far beyond the ~35% it costs the renderer). Run headless with `MFB_CANVAS_GPU=0`, so
there is no graphics thread and no render. Time with `datetime::monotonicNanos` around
loops of 10,000, 3 repetitions, and take the median.

- [ ] **A1 Incremental ladder.** One probe per row, each adding one step to the previous row. Report ns per item.
      | row | loop body |
      | --- | --- |
      | 0 | empty loop with the index arithmetic |
      | 1 | + 8 × `math::cos`/`math::sin` |
      | 2 | + building the 4 `canvas::Point` records into locals (no list) |
      | 3 | + `ring()` as written (4 appends from `[]`) |
      | 4 | + `ring()` as a 4-element list literal `[Point[...], ...]` |
      | 5 | + `canvas::fill(color::rgba(...))` |
      | 6 | + constructing the `canvas::Polygon` (item discarded) |
      | 7 | + `items = collections::append(items, it)` |
      | 8 | + dropping last frame's `items` (the `MUT scene = …` reassignment) |
      | 9 | the same shapes with `canvas::Rectangle` / `canvas::Line` (fixed-width: no points list) |
      Row 9 separates "a variable-width item" from "any item". It is the key split between (b) and (c).
- [ ] **A2 Allocation census.** Count `_mfb_arena_alloc` / `_mfb_arena_free` / `copy_flat_block` / realloc calls per item for rows 3, 4, 6, 7 and 8. Use either:
      - `mfb build -ncode` and read the loop's `.nir`/`.ncode`, citing each allocation to the lowering that emits it; or
      - a counting build of the arena, kept in /tmp.
      The question for each allocation: which source construct emits it, and does the value need it?
- [ ] **A3 Profile.** Run `sample` (or Instruments Time Profiler) against the release worker thread of the full stress program. Attribute self time to: the arena alloc/free paths, memcpy, the list grow arm, trig, and user code. Cross-check against A1's ladder; if they disagree by >20%, find out why before continuing.
- [ ] **A4 Floor.** Write the same loop in C (`/tmp`, `clang -O2`) with the same data layout: a malloc'd 4-point array per item, and an array of 10k tagged items. Also write it with one arena bump per item. This gives the floor for a value-semantics program that allocates per item. Distance from that floor is (a) or (b); the floor itself is (c).
- [ ] **A5 wind.** The same split for `examples/wind`'s `frameScene`. Time `mapItems`, `trailItems`, the merge loop, and the simulation step separately, per frame, in release, and count items of each kind. This is what decides whether G3 (wind at 60 fps) is a codegen problem or a program-structure problem.

Deliverable: the table below, filled in, with the command behind each number.

| step | ns/item | allocations/item | bin (a/b/c) | evidence |
| --- | --- | --- | --- | --- |

## Phase B — try the removals (prototype in /tmp; keep only numbers)

For each (a)/(b) item Phase A finds, prototype the smallest change that removes it, and re-run A1. Candidates to confirm or kill, each a hypothesis until measured:

- [ ] **B1 Moves into constructors and `append`.** Does `canvas::Polygon[points := ring(...)]` copy the returned list, even though a call result has no other owner? Does `items = append(items, it)` deep-copy `it`, even though `it` is dead after the line? plan-134-C's last-read analysis moves on bind/assign/return; check whether it covers constructor fields (`lower_value_stored`) and the in-place append item operand (`.ai/collections.md` "An accumulator threaded through a helper"). If it doesn't, the fix belongs in the compiler and changes no semantics. That is bin (a).
- [ ] **B2 List growth from `[]`.** 4 appends to an empty list is 1 allocation plus N grows. Measure the grow arms' first capacities. Try:
      - a first-grow capacity of 4 (or ≥ a small constant);
      - a literal-count presize when the loop bound is a constant.
      Compare with row 4 (the literal).
- [ ] **B3 Reusing last frame's list.** A program that keeps `items` and updates it in place (`items = collections::set(items, k, newItem)`) instead of rebuilding it. Does the in-place `set` arm apply to a `List OF DrawItem` (variable-width element)? What does it cost against rebuild + drop? If it's cheap, this is a pattern to document on the man page, not a compiler change.
- [ ] **B4 Drop cost.** Row 8 isolated: freeing 10k items, each with a nested list. If it's large, check whether the old scene could be freed in bulk (arena region per frame) instead of per node. That would be a runtime change, bin (b).
- [ ] **B5 Allocator fast path.** If A3 puts most time in `_mfb_arena_alloc`/`free` themselves rather than in how many times they're called, measure a size-class freelist fast path. It would be a runtime change for every program, not canvas-specific.
- [ ] **B6 Trig.** If rows 0→1 are material, compare `math::cos`/`sin` with libm, and check that they aren't going through a slow generic path.

Deliverable: for each candidate, before → after ns per item, whether it changes semantics (it must not for bin a), and its blast radius (every program, or canvas only).

### B-native: the native collection code itself (bin a only: same results, faster code)

The removals above ask how many times a collection primitive runs. These ask how fast each run is. Read `.ai/collections.md` first: the in-place table, the copy-insertion rules (bug-601, plan-134), and the fixed-width vs variable-width payload split (bug-621). A speed-up must keep every one of those invariants. Each item below:

- starts from one microbenchmark in /tmp: 1M operations, release, median of 3;
- is compared with the same operation in C, for its floor;
- reads the emitted `.ncode` of the loop to name the instructions that are not needed.

- [ ] **N1 `append` in place, both arms.**
      - The hot path should be one capacity compare, a store, and a count bump. Count the instructions it actually emits for a fixed-width element (`Integer`, `canvas::Point`) and for a variable-width one (`canvas::DrawItem`).
      - Check whether the variable-width arm re-derives `header + capacity × stride` or re-reads header words each call where it could keep them in registers across a loop.
      - Measure the grow arm's geometric factor and its first capacity.
- [ ] **N2 Record and union construction.** `canvas::Polygon[...]` into a `DrawItem`: how many stores, and whether the payload is built in a temporary and then copied into the union (`{tag@0, size@8, record@16}`) instead of built in place.
- [ ] **N3 `copy_flat_block` and the graph copier.** Wherever A2 finds a copy that must stay (a real second owner), measure its throughput against `memcpy` of the same bytes.
      - Is a flat element list copied with one `memcpy` or element by element?
      - Is a nested list re-walked when its block could be copied whole and its interior pointers rebased?
- [ ] **N4 Drop walkers.** Freeing a `List OF DrawItem`: per-element tag dispatch, a free per nested list, then the outer block. Compare with the minimum, one free per block actually owned. Check whether elements with no pointer fields (the fixed-width variants) still go through the walker.
- [ ] **N5 Arena allocate and free.** The per-call cost of `_mfb_arena_alloc`/`_mfb_arena_free` for the sizes this workload uses (a 4-point list, a `DrawItem`-sized block). Is there a size-class fast path, and does every call take it? This overlaps B5; B5 asks whether the allocator is the bottleneck, N5 asks how fast one call is.
- [ ] **N6 Reads.** `collections::get`/`getOr` and `FOR EACH` over a `List OF DrawItem`: is the element borrowed read-only (`.ai/collections.md` "get read-only borrow") or copied out? Does an element's field read (`it.x`) load straight from the list's data, or materialise the element first? This is what the program's simulation step and the canvas helpers both pay.
- [ ] **N7 Bounds and error paths.** Are bounds checks and `Result` tag tests hoisted or folded where the loop bound already proves them (`WHILE i < len(xs)`)? Count them in the loop's `.ncode`. Only remove a check that is provably redundant; an out-of-range access must still raise the same error.
- [ ] **N8 Float and trig calls.** `math::cos`/`sin`/`floor` and `toFloat`: is the call inlined, or a full runtime call with result-tag handling each time? Compare with a direct libm call.
- [ ] **N9 Bulk operations.** `collections::append(list, sublist)`, `mid`, `slice` and `concat` on a variable-width list: is it one `memcpy` of the data region plus an offset fix-up, or element by element?

For each N item, the output is:

- ns per operation, before and the prototype's after;
- the `.ncode` delta;
- which programs gain: every collection user, not only canvas, which is why these are worth doing even outside this bug;
- the tests that pin the behaviour it must keep: `tests/runtime/rt_list_append_growth_bounds.rs`, `rt_inplace_*` and the byte-identity goldens, which WILL move on a codegen change and must be regenerated only after the full suite, per AGENTS.md.

A finding here becomes its own plan letter. None of it lands inside the spike.

## Phase C — program-structure and API items (measure what already exists before inventing anything)

The user's rule: keep the API "simple enough for anyone". Existing features come first; any new surface must be one concept a beginner can use without learning the renderer.

- [ ] **C1 Groups for static content** (`canvas::setGroup` + `canvas::Group`).
      - Measure a 5,000-item static group plus 5,000 moving items against 10,000 presented items. `present` copies a group node (two offsets and a name), not its items. Confirm the worker and graphics costs stay flat in the group's size, and that the graphics thread's geometry cache keeps the group's records across frames.
      - Wind's coastline is static between pan/zoom steps, so this is the first thing to try in wind. A `Group` has only `dx`/`dy` (no scale), so a zoom still means `setGroup` again. That is fine if zoom is rare; measure the re-install cost.
- [ ] **C2 A transform on `Group`** (API addition, one field). If C1 shows zoom re-installs matter, measure a `Group` that takes a `canvas::Transform`: pan and zoom a static map for free. Transforms already exist on `Paint`, so no new concept for the user; this is the smallest API change that gives "static geometry, moving camera".
- [ ] **C3 Layers** (`canvas::presentLayers`): one static layer and one dynamic layer. Confirm the unchanged layer costs nothing on the worker (`carriedHashes` takes it as byte-equal) and nothing on the graphics thread. Compare with C1.
- [ ] **C4 Bulk append.** Replace wind's `FOR EACH … append` merge with `collections::append(items, trailItems(v))` (the bulk arm). Measure. If it's material, document it on `mfb man collections append` as the pattern to use.
- [ ] **C5 Fixed-width items.** Where a shape is a quad or a segment, compare `Rectangle`/`Line` (no nested list) with `Polygon` (nested list) end to end. If row 9 shows the nested list is most of the cost, the advice "use `Line` for segments, `Polygon` only for real outlines" is free.
- [ ] **C6 Particle systems** (plan-150, `canvas::ParticleSystem`, if merged): smoke, fire, fog, sparks as one item expanded on the graphics thread. Measure 10k and 50k particles. This is the batch primitive for effects that is already designed, so it's the answer for "smoke simulation" before any new mesh type.
- [ ] **C7 A batched primitive for simulations** (API addition, only if B/C1–C6 leave a gap). Candidates:
      - a `canvas::Lines` (or `Points`) item carrying one flat `List OF Float` plus one paint;
      - a per-vertex colour list.
      That covers flow fields, water surfaces and scientific plots: one allocation per BATCH instead of per segment. Measure what 50k segments cost built as one flat list versus 50k `Line` items. This is the old bug-686 Phase 5 idea, scoped down. Recommend it only with numbers and a one-paragraph man-page sketch showing that a beginner can use it.
- [ ] **C8 Pipelined measurement.** Every C row, and the final numbers, also without `MFB_CANVAS_SYNC`. With the worker and graphics thread overlapping, fps = 1 / max(worker, graphics). This is what a real program gets, and it tells which side to work on next.
- [ ] **C9 Threads.** Can a program build half its scene on a second `threads::` worker and merge? Is the merge copy cheaper than the build it saves? Low priority; only if A shows the build is irreducibly per-item and CPU-bound.

## Phase D — also verify (things bug-686 did not measure, and that could hide a cliff)

- [ ] **D1 Real-window rates.** Everything above is headless. In a real window at full-screen Retina, run the 10k moving row through the direct `CAMetalLayer` present (bug-686 Phase 4) and confirm the same fps. `scripts/test-macapp.sh` Case 3h proves the path is taken, not the rate.
- [ ] **D2 Frame pacing, not just average fps.** Record per-frame worker and graphics times (`MFB_CANVAS_STATS` phase timers, debug) and report p50/p95/max. A compaction frame (`geoCompactions`) or a glyph eviction that spikes to 30 ms is a visible stutter even at a 60 fps average.
- [ ] **D3 Memory steady state.** Run 10k moving items for 10 minutes. RSS must plateau: arena, geometry store capacity (now kept across compaction), and the retirement list.
- [ ] **D4 Mixed scenes.** Text-heavy HUD + 5k sprites (`Picture`) + 2k polygons. These are the kinds that still take the MFBASIC resolve path on the graphics thread (Text, Picture, Group, transformed and gradient items). Measure them against the native kinds. Where they are 10× the native kinds, list them for a follow-up.
- [ ] **D5 Large polygons moving.** A 4,000-edge polygon translated every frame: geometry rebuild plus band-index rebuild per frame. Is it in budget?
- [ ] **D6 Resize under load.** 10k items while the window is dragged. No software fallback, no leaked frames.
- [ ] **D7 Debug-build penalty.** Report the release/debug ratio for the program side, so a user judging speed on `--debug` is told on the man page what to expect.

## Output

- The Phase A table, with every number sourced.
- For each bin (a) finding: a bug doc, or a plan letter if it's a codegen project. That includes every B-native (N1–N9) item with a measured win, ranked by ns saved per item in the 10k workload.
- For bin (b): a design note on the language feature, with the measured win and what it would change for users.
- For bin (c) and the Phase C items: a recommendation for the canvas guide.
  - Which patterns to use: groups for static content, layers, fixed-width items, bulk append, particle systems.
  - Whether `Group` transforms or a batched primitive earn an API addition.
- A one-line answer to the question this spike exists for: **can a plain `present(List OF DrawItem)` program reach 10k moving items at 60 fps, and if not, what is the smallest change that gets it there?**

## Non-goals

- Changing value semantics. Any bin-(b) remedy is written up, not implemented, in this spike.
- The renderer. bug-686 owns it; any renderer finding here goes to a new bug doc.
- Vulkan and other targets. The measurements are macOS Metal, like bug-686.

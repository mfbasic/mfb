# plan-175-C: Adjacent group nodes draw as one instanced draw

Last updated: 2026-10-13
Effort: medium (1h–2h)
Depends on: plan-175-B

After plan-175-B every draw-list row is also a node in a per-pass node table, and a row
with repeat `R` draws `count * R` instances, reading node `row + local / count` for its
offset and matrix. This plan writes `R > 1`: a run of ADJACENT rows that draw the same
blocks with the same pipeline -- which is what consecutive `canvas::Group` nodes naming
one group produce -- becomes one drawn head row of repeat `R` followed by `R - 1`
node-only rows of repeat 0. It is an MFBASIC change to the draw-list walk and the pass
walk; neither emitter nor shader changes.

The single outcome: `benchmark/canvas`'s `group fill 10000` draws 2 calls a frame
instead of 10,001 (its 10,000 nodes and the marker), with every frame's pixels
unchanged.

References:

- `planning/completed/plan-175-B-canvas-draw-node-table.md` — the row format (word 13, repeat),
  the node table, and the shaders' `local % count` / `local / count`.
- `planning/completed/plan-175-A-canvas-geometry-identity-split.md` — §2.4 and decision 5.
- `.ai/canvas-threading.md` §10 (the draw list and passes).
- `benchmark/canvas/README.md` — the `group` rows, `profDraws` and `.sums`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| ~~plan-175-B is complete and archived~~ plan-175-B's node table, run and shaders have landed on every backend (its Phases 1–3 resolved), Correction 2 | `grep -c "^- \[ \]" planning/completed/plan-175-B-canvas-draw-node-table.md` → 0 | MET once B's Phase 2 and 3 commit lands (checked 2026-10-25 before this letter's commit) |

Everything below is written against the world where that holds.

## 1. Goal

- At the end of `helper_render.rs:__canvas_sceneDrawsWalk`, every maximal run of
  adjacent rows whose words 0, 1 and 4 (instance base, instance count, blend mode) are
  equal becomes: the first row with word 13 = `R` (the run's length), and the other
  `R - 1` rows with word 13 = 0. Their node words are untouched, so node `k` of the run
  is still row `k`'s offset and matrix. `R` is at most `MAX_FRAME_NODES`; a longer run
  is cut into runs of at most that many.
- The pass walk keeps a head row and its `R - 1` followers in one pass.
- `group fill 10000`: `profDraws` 2 a frame (`.sums` `group_fill_10000_draws = 2`), and
  every row's pixel hash unchanged.

### Non-goals (explicit constraints)

- No row is moved or reordered, and no two rows that are not adjacent are merged, so
  draw order is the list's order (§2, "Why adjacency is the whole rule").
- No emitter, shader, row format or node-table change (plan-175-B owns those).
- The native flat list (`canvas::sceneDrawsFlat`, a frame with no group) is untouched:
  its rows have strictly rising bases, so no two adjacent rows are equal and the merge
  would change nothing.

## 2. Current State

- `helper_render.rs:__canvas_drawGroup` lays out each group once per frame
  (`__canvas_memoGroup`, keyed by slot and `AutoPaint` variant) and pushes one run of
  rows per NODE at that node's offset and matrix (`__canvas_pushDraw` →
  `__canvas_pushOneDraw`). Two nodes naming one group therefore push rows with equal
  words 0, 1 and 4 and different node words.
- `__canvas_sceneDrawsWalk` walks the scene in order, so consecutive group nodes of one
  group produce adjacent rows when the group's own items form ONE run (one blend mode,
  no `Text` split, no nested group): `group fill 10000`'s rows are 10,000 such rows and
  the marker's.
- Since plan-173, `__canvas_pushDraw` writes rows only for blocks in the viewport,
  tested per block under each node's own offset and matrix. Two consequences for the
  merge:
  - **A node wholly offscreen writes no row.** The visible nodes on either side of it
    become adjacent rows and merge. That is correct, because the culled node drew
    nothing between them, so instance order is still draw order.
  - **A node partly offscreen writes only its visible blocks**, as one or more shorter
    runs whose words 0 and 1 differ from a whole node's. It merges with nothing, which
    is correct but merges less. A large map group panned across an edge is the case
    where this shows: its nodes on the edge stay separate draws.
- `__canvas_passNext` adds rows one at a time; a row whose blocks are already in the
  pass is found by `__canvas_passFind` and costs no blocks. After plan-175-B it also
  counts rows against `__CANVAS_PASS_MAX_NODES`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Draw rows a frame, `group fill 10000` | 10,001 | `grep group_fill_10000_draws benchmark/canvas/baseline/canvas.sums` |
| Draw rows a frame, `group fill 100` | 101 | `grep group_fill_100_draws benchmark/canvas/baseline/canvas.sums` |
| Encode median, `group fill 10000` | 2,221 µs | `awk '/^group fill 10000:/{s=1} s&&/encode/{print;exit}' benchmark/canvas/baseline/canvas.log` |

### Verified properties

- **Why adjacency is the whole rule.** A head row of repeat `R` draws instances in node
  order: all of node 0's instances, then node 1's, and so on (`node = row + local /
  count`, plan-175-B §1), which is the order the `R` separate rows drew them in. A
  group whose run is SEVERAL rows (a blend split, a `Text`, a nested group) pushes
  `a0 b0 a1 b1` for two nodes; no two adjacent rows are equal there, so nothing merges
  and `b0` still draws before `a1`. Read: `__canvas_pushDraw` splits a node's run into
  rows in order and nothing between two nodes' rows is reordered.
- **Draw order within one instanced draw is instance order on every backend.** That is
  what the paragraph above relies on, and Phase 1's overlap test proves it against the
  software oracle on Metal, box 2226 (lavapipe) and box 206 (Intel UHD 630): two nodes
  of a two-child group overlapping so that node 1's first child covers node 0's second,
  which is wrong in exactly the overlap if the GPU orders instances any other way.

## 3. Design Overview

One pass over the finished row list in `__canvas_sceneDrawsWalk` sets word 13; one rule
in `__canvas_passNext` keeps a head with its followers. The risk is ordering, and it is
removed by construction (only adjacent rows merge) and proven by the overlap test. The
gate is pixels (`benchmark/canvas` `.sums`, the oracle suites, the GPU rows) plus the
draw count.

Rejected alternatives:

- **Merging every node of a group in the frame, wherever it sits.** It reorders draws
  against the items between the nodes, which is a wrong picture wherever they overlap.
- **Building merged rows inside `__canvas_drawGroup` as nodes are pushed.** The walk
  would have to look back across calls and nested groups; a pass over the finished list
  sees exactly the adjacency that decides correctness.

## 4. Detailed Design

- `helper_render.rs`: `__canvas_mergeRepeats()`, called as the last step of
  `__canvas_sceneDrawsWalk`. It walks the rows once, comparing each row's words 0, 1, 4
  with the head of the current run; a run ends at the first unequal row or at
  `__CANVAS_PASS_MAX_NODES` rows. It writes word 13 = run length into the head and 0
  into the rest (`collections::set` on `__CANVAS_DRAWS`, no new list).
- `__canvas_passNext`: a row of repeat `R > 1` is admitted only if the pass's rows plus
  `R` fit `__CANVAS_PASS_MAX_NODES`; it and its `R - 1` followers are then added
  together (each follower through `__canvas_passDraw`, which copies its words; its
  blocks are the head's, already in the pass). A follower is never the first row of a
  pass.
- `__canvas_drawsText` already prints `:repeat` (plan-175-B).

## Compatibility / Format Impact

- No API, format or pixel change. Draw calls a frame fall for scenes with adjacent
  nodes of one group; `profDraws` and `draws=` report it.

## Phases

### Phase 1 — Merge adjacent rows, and keep a run in one pass

- [x] `helper_render.rs`: `__canvas_mergeRepeats` (§4), called from
      `__canvas_sceneDrawsWalk` (`helper_render/gpu_draw_list.rs`).
- [x] `helper_render.rs:__canvas_passNext`: a head and its followers enter a pass
      together (§4) — and a head a pass boundary cuts draws solo with its followers
      (`helper_render/gpu_passes.rs`, Correction 3).
- [x] (added, Correction 3) `profDraws` counts the draws a pass issues (`OFF_DRAWN`,
      `app/metal.rs:emit_profile_draws`), not its rows.
- [x] Tests, ~~`tests/canvas/rt_canvas_rasteriser.rs`~~ `tests/canvas/rt_canvas_metal/group_merge.rs`
      (Correction 3), each a Metal frame within `GPU_DEFAULT` of the software frame →
      all six pass (`a_thousand_adjacent_group_nodes_are_one_draw`: repeats 1,000 then 999
      zeros, `profDraws=1`; `a_merged_draw_keeps_instance_order_where_nodes_overlap`:
      `[2, 0]`, (150, 120) red; `group_nodes_with_an_item_between_them_stay_separate_draws`:
      `[1, 1, 1]`; `a_run_longer_than_the_node_region_is_cut_and_drawn_in_two_passes`:
      heads 65,536 and 4,464, `profPasses=2`;
      `culled_nodes_merge_across_and_a_half_visible_node_stands_alone`: `[2, 0, 1, 1]`;
      and the added `a_merged_run_cut_by_a_pass_boundary_draws_each_node_alone`):
      - 1,000 nodes of a one-rectangle group: `draws=` is one row of repeat 1,000 and
        999 of repeat 0, and the frame equals the software frame;
      - the overlap test of §2 (two nodes of a red-then-blue group, node 1's red over
        node 0's blue): equals the software frame;
      - nodes of one group with another item between them: no merge across it
        (`draws=` shows separate heads) and the frame equals the software frame;
      - 70,000 nodes of one group: the run is cut at 65,536, the frame draws in two
        passes, and equals the software frame;
      - culling around a merge (plan-173): nodes in view with one node wholly
        offscreen between them merge across it into one head, and a node half across
        the right edge does not merge with the whole nodes beside it. `draws=` shows
        both, and the frame equals the software frame.
      Every node of the other four cases is placed wholly inside the 900×640 surface:
      an offscreen node writes no row, and would change the counts asserted.

Acceptance: adjacent identical rows draw as one call, every frame unchanged.
  Check: `cargo test --release --test rt_canvas_rasteriser` → pass (est. 4 min; the
  draw list and the pass walk are what changed, and this suite reads `draws=` and
  compares with the oracle).
  Check: `./benchmark/canvas/run.sh --only '^group '` then `compare.py` → `.sums`
  pixels unchanged for every `group` row, `group_fill_10000_draws = 2` and
  `group_fill_100_draws = 2` (est. 1 min).
  Check: `scripts/test-canvas-vulkan.sh` on box 2226 and `scripts/test-canvas-gpu-rows.sh
  <mfb> --target windows-x86_64` on box 206 → the overlap and 70,000-node rows match
  (instance order on both real Vulkan drivers in reach).

**MET 2026-10-25**, against the base compiler at `339f8e8fc`:
- `cargo test --release --test rt_canvas_rasteriser --test rt_canvas_metal` →
  rasteriser 111 passed (2 ignored), metal 81 passed (the six merge cases among them);
- the benchmark's 87 rows headless on Metal (plan-175-B Correction 3) → 87 last-frame
  hashes equal to the base's; `group fill 10000` and `group fill 100` draw 2 calls a
  frame (`profDraws` 6 over 3 frames, against 30,003 and 303 at the base);
  `group fill 1` 2 against 2;
- `scripts/test-canvas-gpu-rows.sh <mfb> --target windows-x86_64 --keep` and `--target
  linux-aarch64 --box 2226 --keep` with this letter's four rows added (`nodes65537`,
  `mergeoverlap`, `mergecut`, `mergesolo`; Correction 3) → 274 `ok:` and no `FAIL` on
  each box, the four new rows within `worst=0` of the oracle on both drivers, and every
  one of the 124 kept frames per box equal by `cmp` to the base's.
Commit: 0040e128b

### Phase 2 — Re-baseline and docs

- [x] `./benchmark/canvas/run.sh` in full; `compare.py` against ~~the baseline~~ main's
      tip in the same session (Correction 4): every `.sums` pixel hash unchanged, the
      `group` rows' draws and encode moved. Commit the run as the new
      `benchmark/canvas/baseline/` with `compare.py`'s output in its README. → 435
      checksums, 2 changed: `group_fill_10000_draws` 10,001 → 2 and
      `group_fill_100_draws` 101 → 2; `group fill 10000` encode 2,153 → 60 µs.
- [x] `.ai/canvas-threading.md` §10: adjacent group nodes merge; why adjacency is the
      rule; a head and its followers share a pass. ("Adjacent group nodes draw as one
      call", with the solo rule and `profDraws`.)

Acceptance: the baseline records the merge; the docs describe HEAD.
  Check: `./benchmark/canvas/compare.py <run>.log --base <previous baseline>` → `group
  fill 10000` encode below 2,221 µs and draws 2, no other row's checksum changed (est.
  5 min, attended).
Commit: 29f049ff9 (baseline); the §10 notes in fd66b249f

## Validation Plan

- Tests: Phase 1's four runtime cases; every suite comparing GPU frames to the oracle.
- Coverage check: `benchmark/canvas` covers the direct-present path in a real window,
  which the headless cargo suites do not.
- Runtime proof: `group fill 10000` at 2 draws a frame with an unchanged pixel hash.
- Doc sync: Phase 2.
- Final gate (once, after Phase 2): `cargo test --release`, `scripts/artifact-gate.sh
  target/debug/mfb all` (canvas goldens expected to move; no other golden).

## Decisions

- Group-node instancing is its own letter after plan-175-A, built on plan-175-B's node
  table — decided by the owner, 2026-10-13.
- Only adjacent rows merge — the author's call: it is what keeps draw order without a
  dependency analysis, and it covers the measured case.

## Corrections

1. 2026-10-13, before any phase: plan-173 (merged at `bf951f32f`) made the draw-list
   walk cull offscreen blocks per node. §2 records what that does to adjacency, and
   Phase 1 gains a culling case and the rule that the other cases' nodes are in view.
   The goal is unchanged: `benchmark/canvas`'s `group` rows are all in view.
2. 2026-10-25, at the Prerequisites gate: "plan-175-B is complete and archived" — the
   letters are archived together, after the plan's closing gates (plan-175-B Correction
   2), so the row checks what this letter needs: B's node table landed and verified.
3. 2026-10-25, Phase 1: four corrections.
   - **A head whose BLOCKS a pass boundary cuts** was not covered: §4's rule keeps a head
     with its followers by node count, but a group larger than a pass's item region is
     split mid-head by blocks, and a repeat cannot reach into the next pass's node
     table. Such a run's rows draw solo from the cut on (`__CANVAS_PASS_SOLO_END`, set
     where the boundary cuts the head, read by `__canvas_passDraw`). Added a case:
     `a_merged_run_cut_by_a_pass_boundary_draws_each_node_alone` (two nodes of a
     70,000-rectangle group); with the solo rule disabled it FAILS against the oracle.
   - **The frame's node count is its ROWS** (`__canvas_framePasses`), not the sum of
     repeats: a head's followers are rows of their own. plan-175-B Phase 1 summed
     `max(repeat, 1)`, which equalled the row count while every repeat was 1.
   - **`profDraws` counted rows**, "one per draw-list entry"; a node-only row draws
     nothing, so the count would have read 1,000 for 1,000 merged nodes. Metal now counts
     the draws it issues (`OFF_DRAWN`, `emit_profile_draws`), after the draw pass.
   - **The tests are in `rt_canvas_metal`** (`group_merge.rs`), not `rt_canvas_rasteriser`:
     each case compares a GPU frame with the software oracle, which that suite does not
     draw. The Vulkan check runs the same scenes as rows of
     `scripts/test-canvas-gpu-rows.sh` (`nodes65537`, `mergeoverlap`, `mergecut`,
     `mergesolo`); `scripts/test-canvas-vulkan.sh` runs fixed acceptance scenes and has
     no row list to add them to.
4. 2026-10-25, Phase 2: the comparison is against main's tip, not the committed
   baseline. That baseline was run at `840fa4e18` (2026-10-14), and main changed the
   renderer's pixels and buffer bytes since (`rect_fill_1_bytes` 736 → 608 among many),
   so a run of any later compiler differs from it on rows this plan never touched.
   Main's tip `ba92007cd`, run windowed in the same session, is the base that isolates
   this plan: 2 of 435 checksums changed, the two intended. The acceptance's "no other
   row's checksum changed" is checked there. `run.sh` itself had to be fixed first
   (`a9edb077b`): since bug-1087 it stopped at its first row with any compiler.

## Summary

The merge is a few lines of MFBASIC because plan-175-B put every draw's offset in a
node table the shaders index by instance. The only way to get it wrong is ordering,
and adjacency rules that out by construction; the overlap test proves the GPUs honour
instance order on every backend in reach.

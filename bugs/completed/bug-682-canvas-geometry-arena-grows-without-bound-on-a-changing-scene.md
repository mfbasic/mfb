# bug-682: the canvas geometry arena grows without bound when a scene's items change every frame

Last updated: 2026-09-23
Effort: large (3h–1d)
Severity: HIGH
Class: Memory-safety

Status: FIXED (881863fe6)
Regression Test: `tests/canvas/rt_canvas_geo_arena.rs`

A `Mode.Canvas` program that re-presents a scene whose items have **new content
every frame** — the ordinary shape of any animation — grows its resident memory
without bound until the OS kills it. It is not a slow leak: a scene of ~660
`canvas::Line` items, presented at frame rate, reached 3.4 GB in 18 seconds and
was still accelerating.

The geometry cache holds at most 256 entries and evicts LRU, so the cache
*index* is bounded. The backing store `__CANVAS_GEO_DATA` is not: nothing ever
reclaims the floats an evicted entry owned, and every miss additionally copies
the whole arena. So the arena grows by one item's geometry per miss, and the
per-miss cost grows with it — quadratic time on top of unbounded space.

**The single correct behavior a fix produces:** presenting a scene of N items
whose content changes every frame holds steady-state memory bounded by the
cache capacity and the scene size, not by the number of frames presented. A
program can animate indefinitely at constant RSS.

This is dangerous well beyond a performance complaint: it is reached by the
*intended* use of a retained canvas for animation, it has no diagnostic, and it
ends in an OOM kill of the user's machine rather than an error.

References:

- `src/codegen/builtins/canvas/helper_geometry.rs:105` `__CANVAS_GEO_DATA`,
  `:108` `__CANVAS_GEO_CAPACITY`, `:944` `__canvas_geoEvict`,
  `:968` `__canvas_geometryFor`, `:1007` the per-miss arena copy.
- `mfb man canvas` — "Animated content does call it every frame, which is why
  the runtime caches each item's geometry on a content hash: re-presenting an
  item that did not change is free." The cache is documented for the
  *unchanged* case; this bug is what the changed case costs.
- `.ai/canvas-threading.md` §4 — the redraw triggers; the program drives
  animation by re-presenting, which is exactly the path that leaks.
- `planning/plan-150-*` — `canvas::ParticleSystem` expands N particles on the
  graphics thread specifically so a program need not present N changing items.
  It reduces how often this bug is hit; it does not fix it, and a program
  animating any other way still hits it.
- Found while writing `examples/wind`, which draws wind streamlines as
  per-frame `canvas::Line` items.

## Failing Reproduction

`examples/wind` at the commit this bug was written against, with its particle
count pinned to 60 (≈660 `canvas::Line` items per frame, each with new endpoint
coordinates every frame), sampling RSS every 2 s:

```sh
mfb build examples/wind
./examples/wind/build/wind.app/Contents/MacOS/wind &
while :; do ps -o rss= -p $!; sleep 2; done
```

- Observed:

  | t | RSS |
  | --- | --- |
  | 2 s | 706 MB |
  | 4 s | 777 MB |
  | 6 s | 854 MB |
  | 8 s | 927 MB |
  | 10 s | 999 MB |
  | 12 s | 1072 MB |
  | 14 s | 1143 MB |
  | 16 s | 1474 MB |
  | 18 s | 3460 MB |

  Growth is steady and then accelerates — the signature of an arena that grows
  while each addition copies all of it. At the example's intended 523 particles
  (≈5,750 items/frame) the same program passed 2 GB within 2 s and, before the
  watchdog existed, drove the host into an unrecoverable swap storm.

- Expected: RSS reaches a steady state within a few seconds and stays there for
  as long as the program runs.

Contrast case, same program, same frame rate, one line changed — the
per-frame items removed, leaving only the ~95 static map polygons (whose
content is identical frame to frame, so every probe is a cache hit): **stable
for the full run, no growth.** That bounds the bug precisely to items whose
content changes.

| Environment | | Result |
| --- | --- | --- |
| macos-aarch64 | release, software raster (`gpuSelected=FALSE`) | fails ✗ |
| Linux / Windows, GPU backends | not measured | the cache is backend-independent MFBASIC helper source, so expect ✗ everywhere |

## Root Cause

Two independent defects in `helper_geometry.rs`, either of which alone would be
survivable:

**1. The arena is never reclaimed.** `__canvas_geoEvict` (`:944`) drops the
evicted slot from the four *index* lists — `__CANVAS_GEO_HASHES`, `_OFFSETS`,
`_COUNTS`, `_LASTUSED` — and nothing else. The floats that entry owned inside
`__CANVAS_GEO_DATA` (`:105`) stay allocated forever. The code says so
deliberately (`:114`, `:842`): offsets held by the frame currently rendering
must stay readable, so "compacting `__CANVAS_GEO_DATA` would move every other"
entry's offset. That reasoning is sound for *compaction within a frame*; what is
missing is any reclamation at all, at any point — including at a frame boundary,
when no offsets are live.

**2. Every miss copies the whole arena.** `__canvas_geometryFor` (`:1007`):

```basic
MUT buffer AS List OF Float = __CANVAS_GEO_DATA
' … append this item's header and tail …
__CANVAS_GEO_DATA = buffer
```

The comment above it explains the local is there to avoid appending straight
into the global (which would copy per *element*). But `MUT buffer =
__CANVAS_GEO_DATA` is itself a copy of the entire arena — the global is read and
then reassigned, so value semantics give a copy, not a move. So each miss costs
O(arena), and the arena only grows.

Together: with 660 changing items and a 256-entry cache, every item is a miss
(the cache thrashes completely), so each frame performs ~660 full-arena copies
while adding ~660 items' worth of floats that are never freed. Frame *k* copies
an arena of size O(k), which is the acceleration in the table.

The contrast case is immune because 95 identical-content items against a
256-entry cache are all *hits*: `__canvas_geometryFor` returns at the probe loop
(`:987`) and never reaches either defect.

## Goal

- A program presenting N changing items per frame reaches steady-state RSS,
  bounded by cache capacity × max entry size, independent of frames presented.
- A cache miss costs O(entry), not O(arena).
- A new test fails on today's code and passes after: present a changing scene
  for a few hundred frames and assert the arena length (or RSS) is flat.

### Non-goals (must NOT change)

- **No change to what any scene draws.** This is storage management; every
  rendered frame must be byte-identical to today's, and the canvas reference
  images must not move.
- **Do not break the live-offset invariant** (`:114`). A frame resolves every
  item's offset before drawing any of them, so an offset can outlive its cache
  entry within a frame. Any reclamation must happen where no offset is live, or
  must keep `__CANVAS_GEO_LIVE` honest. Do not "fix" this by compacting mid-frame.
- **Do not simply raise `__CANVAS_GEO_CAPACITY`.** A bigger cache delays the
  growth and makes each miss copy more; it does not bound anything.
- **Do not fix it by declaring animation unsupported** or by documenting a
  ceiling on item count. The documented contract (`mfb man canvas`) invites
  per-frame presents.
- Glyph pinning and eviction renumbering (`:115`, `:740`) must keep working.

## Blast Radius

- `helper_geometry.rs:__canvas_geometryFor` / `__canvas_geoEvict` — **the bug**.
- Every `Mode.Canvas` program that animates — `examples/wind` (works around it,
  see below), and any future one. Static scenes are unaffected.
- `planning/plan-150-*` `canvas::ParticleSystem` — **interacts; verdict recorded
  Phase 1.** Plan-150 is written but **not landed** (`grep -rn Particle src/
  --include='*.rs'` → no hits), so nothing in the tree reaches this bug by that
  route today. Its design does route particles through this cache: §3 piece 4 /
  Phase 4 emit each particle "through the ordinary `__canvas_geometryFor` path
  precisely so that B measures the real thing", and §2 lists "the geometry
  cache's behaviour under N distinct per-particle transforms" as UNVERIFIED and
  letter B's opening measurement. N particles have N distinct hashes against a
  256-entry cache, so a particle system is exactly the changing-content case —
  it would have hit this bug by the second route, and letter B would have
  measured this bug rather than the cache. This fix bounds that path in advance;
  B's measurement is now of the cache.
- `__CANVAS_GEO_LIVE` (`:117`) — already exists to track offsets a frame holds;
  a reclamation design should reuse it rather than invent a second mechanism.
- The glyph cache (`__canvas_glyphEntry`, eviction renumbering) — separate
  store, separate eviction, **unaffected**, but it is the precedent to copy: it
  already evicts and renumbers, which is what the geometry arena does not do.

## Fix Design

Two changes, independently landable, smallest first:

1. **Stop the per-miss arena copy.** Append into `__CANVAS_GEO_DATA` through a
   shape the in-place append arm accepts, so a miss costs O(entry). If the
   global-append arm cannot serve it, hold the arena in a local for the whole
   of a present and write it back once, rather than once per item.

2. **Reclaim on eviction.** The arena needs a free list or generational reset.
   The least invasive shape that respects the live-offset invariant is a
   **frame-boundary reset**: at the start of a present, when no offsets are
   live, drop entries whose `_LASTUSED` predates the previous frame and rebuild
   the arena tightly, renumbering the surviving offsets — exactly what glyph
   eviction already does for glyph indices.

Rejected: reference-counting offsets (needs a release path on every draw);
never evicting and hoping capacity is enough (unbounded by construction);
capping the arena and failing the frame (turns a leak into a visual defect).

Note that (1) alone converts the failure from "OOM in 18 s" to "OOM in minutes"
— it fixes the time complexity but not the space. (2) is the actual fix. Land
(1) first because it is small and makes (2)'s measurements legible.

## Phases

### Phase 1 — failing test (no behavior change)

- [x] Add a canvas test that presents a scene of N items with new coordinates
      each frame for ~300 frames and asserts `len(__CANVAS_GEO_DATA)` (via the
      `MFB_CANVAS_STATS` line, extending it if needed) is flat after warm-up.
      Confirm it fails today.
- [x] Record whether plan-150's expansion path probes the same cache; write the
      verdict into Blast Radius.

Acceptance: the test fails showing unbounded arena growth. **Met.**
`tests/canvas/rt_canvas_geo_arena.rs` failed with `floats=42300` at frame 15 and
`floats=84600` at frame 30 — exactly linear in the frame number while
`entries=256` stayed pinned, which is the documented mechanism and not merely
the symptom.

Deviation: **30 frames, not 300.** The unfixed cost is quadratic, so 100 frames
of this program took 75 s and 21.6 GB peak RSS; 300 would not have completed on
a normal machine. Thirty separates the two behaviours by 2x, and the quantity is
a deterministic count of floats rather than a measurement, so there is no noise
for a larger sample to average out. The `MFB_CANVAS_STATS` line already carried
`floats=`; it gained `geoCompactions=` for the second test.

Commit: 881863fe6

### Phase 2 — O(entry) misses

- [x] Remove the whole-arena copy at `helper_geometry.rs:1007`.

Acceptance: frame time stops growing with frame number. **Met**, and the arena
copy is gone outright rather than merely bounded: the miss now appends straight
into `__CANVAS_GEO_DATA`.

The Fix Design's fallback ("hold the arena in a local for the whole of a
present") was not needed, because **the premise the local rested on is stale**.
The comment at `:1002` said `collections::append` is in-place only for a local,
so appending into a global copies per element. That was true when it was
written and is not now: `x = collections::append(x, e)` on a module-level global
is site **S2** of the in-place self-update table (plan-121-A / plan-142,
`.ai/collections.md`). Measured before relying on it — 2,000,000 appends into a
global, 0.05 s user, linear in N. The element is hoisted into a `LET` first so
the operand cannot be something the S2 gate `G-global-operand` declines on.

Commit: 881863fe6

### Phase 3 — reclamation

- [x] Reclaim evicted entries' floats at a point where no offset is live,
      renumbering survivors; keep `__CANVAS_GEO_LIVE` correct.

Acceptance: Phase 1's test passes; RSS is flat. **Met.** `__canvas_geoCompact`
rebuilds `__CANVAS_GEO_DATA` from what the surviving slots own and renumbers
`__CANVAS_GEO_OFFSETS`, called from the top of `__canvas_sceneOffsets` — the one
point in a frame where no offset is live, which is also where
`__CANVAS_GEO_LIVE` is cleared, so the two invariants are re-established
together. The frame-boundary reset of the Open Decision, as recommended.

That point is safe because **nothing carries an offset across a frame**: the
damage diff's `__canvas_rememberScene` stores the *bounds values* it read
through each offset, not the offsets. The old design's "never compact" rule was
right about the where — compacting mid-frame draws one item's geometry for
another — and wrong only in concluding there was nowhere.

Gated on 2x slack rather than running unconditionally, which is load-bearing
rather than a micro-optimisation: an all-hit static scene holds an arena that is
*exactly* its live entries, so an ungated pass would add an O(arena) rebuild to
every frame of every static canvas program — a cost the bug itself never had.
`an_all_hit_scene_never_pays_for_reclamation` pins that. The same gate spaces an
animating scene's rebuilds geometrically, so their amortised cost per miss is
O(1).

Commit: 881863fe6

### Phase 4 — validation

- [x] `cargo test --test 'rt_canvas_*'` green, including every golden reference
      image unchanged (this fix must not move a single pixel).
- [x] Re-run the reproduction at the documented scale and hold steady RSS.
      **Deviation: `examples/wind` is not in the tree** — it was never
      committed, and this doc is the only thing that references it (`ls
      examples/` has no `wind`). The substitute is a standalone program at the
      scale the Failing Reproduction documents: 660 `canvas::Line` items with
      new endpoint coordinates every frame, 600 frames, RSS sampled every 15 s.

      RSS reached 93,936 KB at t+45 s and was **still exactly 93,936 KB at
      t+180 s**, over 306 frames, with `floats=74072` flat from frame 20 through
      frame 600 and a 96.6 MB process peak. The doc's table for the same item
      count reads 706 MB at 2 s and 3.4 GB at 18 s.

All twelve `rt_canvas_*` suites green: golden 6, rasteriser 19, damage 60,
font 2, graphics_thread 24, debug_hooks 11, group_ownership 11, metal 17,
picture 10, present_deep_copy 10, image_decode 8, system_fonts 4, geo_arena 2.
No reference image moved.

Commit: 881863fe6

## Validation Plan

- Regression test: `tests/canvas/rt_canvas_geo_arena.rs` — the arena-flatness
  test, plus `an_all_hit_scene_never_pays_for_reclamation` pinning that a static
  scene never runs the new pass. **Done.**
- Runtime proof: 660 changing `canvas::Line` items, 600 frames — flat at 92 MB
  (Phase 4). **Done**, with `examples/wind` substituted for as recorded there.
- Doc sync: `mfb man canvas` gained a paragraph after the caching one, saying
  what a *changed* item costs and that a program can animate indefinitely at a
  steady size (`src/codegen/builtins/canvas/mod.rs`, `MODULE_DESC`). **Done.**
- Full suite: `cargo test` green, and `scripts/artifact-gate.sh <mfb> all`
  **1495 tests, 2116 goldens, 0 diffs** after regenerating the seven
  `syntax/app/app-mouse-surface` goldens the canvas helper change moves — the
  one fixture `.ai/testing-gates.md` names as the canvas change-sentinel, and
  the only thing in the tree that moved. Its `.ast` was **byte-identical**, so
  the front end held still and the change is lowering-only.

  The `.ir` delta was inspected with line numbers normalised before
  regenerating: 116 lines, and all of it accounted for — the new
  `#CANVAS_GEO_COMPACTIONS` global, the new `#canvas_geoCompact` sub, its one
  call site in `sceneOffsets`, the `buffer` local replaced by direct
  `assignGlobal` appends in `geometryFor`, and `ErrorLoc` source lines shifted
  by the insertion (`4568` → `4642`). Nothing unexplained.

## Open Decisions

- Frame-boundary reset vs. a free list — recommended: **frame-boundary reset**,
  because it reuses the point at which no offset is live and mirrors the glyph
  cache's existing renumbering, rather than adding a second allocator. (§Fix Design)

  **Settled as recommended.** `__canvas_geoCompact` at the top of
  `__canvas_sceneOffsets`, gated on 2x slack. No second allocator, no release
  path on any draw.

## Summary

The risk concentrates in Phase 3: reclamation has to respect the live-offset
invariant the current design leans on, and getting it wrong reads freed
geometry rather than merely leaking. Phase 2 is small and safe. Nothing about
what a scene draws changes; the canvas reference images are the guard that
proves it.

## STATUS: FIXED (881863fe6)

Both defects fixed; every phase met its acceptance. Measured, 60 changing
`canvas::Line` items over 100 frames:

| | arena at frame 100 | peak RSS |
| --- | --- | --- |
| before | `floats=282000`, growing 2820/frame | 21.6 GB |
| after | `floats=14852`, flat from frame 10 | 45 MB |

The canvas reference images did not move: all thirteen `rt_canvas_*` suites
green, and the tree-wide artifact gate reports 0 diffs over 2116 goldens.

Deviations from the written plan, each recorded in full at its phase:

1. **Phase 1's test is 30 frames, not ~300.** The unfixed cost is quadratic, so
   300 frames would not have completed. The quantity is a deterministic count of
   floats, so there is no noise a larger sample would average out.
2. **Phase 2 needed no fallback, because the premise the bug doc inherited from
   the code was stale.** The comment at `:1002` said appending into a global
   copies per element; that has not been true since the in-place self-update
   table gained site S2 (plan-121-A / plan-142). Measured before relying on it —
   2,000,000 appends into a global, 0.05 s user, linear in N. So the miss appends
   straight into the arena and the "hold it in a local for the whole present"
   fallback was unnecessary.
3. **Phase 4's `examples/wind` does not exist in the tree.** Substituted a
   standalone program at the documented 660-item scale; see Phase 4.

Two things worth knowing that the fix revealed:

- **The gate's canvas blindness has exactly one hole, and this change found it.**
  `.ai/testing-gates.md` records that no `tests/byte-identity/` fixture emits the
  canvas runtime, so a canvas change can move zero hashes there. It moved seven
  goldens in `tests/syntax/app/app-mouse-surface`, which is the correction that
  file already carries from bug-484. A canvas MFBASIC helper edit should expect
  exactly that fixture and nothing else; anything more is a real blast radius.
- **Compaction frequency scales with how far the scene overruns the cache, and
  that is the intended shape rather than a residual cost.** At 60 items against
  256 slots it runs about once every five frames; at 660 it runs every frame,
  because one frame's misses alone exceed the 2x threshold. Even then it is one
  O(arena) pass over ~74,000 floats (~590 KB) per frame, against the 660
  whole-arena copies per frame it replaced — and the space is bounded either way.
  A program animating far more items than the cache holds pays a per-frame
  rebuild; if that ever matters, the lever is `__CANVAS_GEO_CAPACITY`, not the
  threshold.

# bug-682: the canvas geometry arena grows without bound when a scene's items change every frame

Last updated: 2026-09-23
Effort: large (3h–1d)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: `tests/canvas/` — a new RSS/arena-growth test; see Phases

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
- `planning/plan-150-*` `canvas::ParticleSystem` — **interacts**. Expansion on
  the graphics thread produces ordinary draw entries, so if those entries probe
  this same cache, a particle system of N moving particles hits this bug by
  another route. Plan-150's cost letter (B) should be checked against this.
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

- [ ] Add a canvas test that presents a scene of N items with new coordinates
      each frame for ~300 frames and asserts `len(__CANVAS_GEO_DATA)` (via the
      `MFB_CANVAS_STATS` line, extending it if needed) is flat after warm-up.
      Confirm it fails today.
- [ ] Record whether plan-150's expansion path probes the same cache; write the
      verdict into Blast Radius.

Acceptance: the test fails showing unbounded arena growth.
Commit: —

### Phase 2 — O(entry) misses

- [ ] Remove the whole-arena copy at `helper_geometry.rs:1007`.

Acceptance: frame time stops growing with frame number; the arena still grows
(Phase 1's test still fails, but linearly rather than quadratically).
Commit: —

### Phase 3 — reclamation

- [ ] Reclaim evicted entries' floats at a point where no offset is live,
      renumbering survivors; keep `__CANVAS_GEO_LIVE` correct.

Acceptance: Phase 1's test passes; RSS is flat.
Commit: —

### Phase 4 — validation

- [ ] `cargo test --test 'rt_canvas_*'` green, including every golden reference
      image unchanged (this fix must not move a single pixel).
- [ ] Re-run the reproduction: `examples/wind` at full particle count holds
      steady RSS for several minutes.
Commit: —

## Validation Plan

- Regression test: the arena-flatness test from Phase 1.
- Runtime proof: `examples/wind` at its intended particle count, RSS sampled
  over 5 minutes, flat.
- Doc sync: `mfb man canvas`'s paragraph on the per-frame present should say
  what the changed case costs once it is bounded.
- Full suite: `cargo test`, and the canvas goldens in particular.

## Open Decisions

- Frame-boundary reset vs. a free list — recommended: **frame-boundary reset**,
  because it reuses the point at which no offset is live and mirrors the glyph
  cache's existing renumbering, rather than adding a second allocator. (§Fix Design)

## Summary

The risk concentrates in Phase 3: reclamation has to respect the live-offset
invariant the current design leans on, and getting it wrong reads freed
geometry rather than merely leaking. Phase 2 is small and safe. Nothing about
what a scene draws changes; the canvas reference images are the guard that
proves it.

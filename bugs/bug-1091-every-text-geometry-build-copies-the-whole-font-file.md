# bug-1091: every `canvas::Text` geometry build copies the whole font file, several times

Last updated: 2026-10-13
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Other (performance)

Status: Open
Regression Test: none yet — Phase 1 below adds one

## Cause (coordinator, bug-staging-24)

- Rule: the font table is read in place, by the table's own load-acquire read path (`.ai/canvas-threading.md` §13: fixed chunks, published name-last, readable without a lock); a text build needs bytes of the font, not an owned copy of the whole file.
- Where: `gen_font_table.rs:lower_font_blob_unchecked` ends in `copy_flat_block` (an owned `List OF Byte` per call), and every text builder calls `__canvas_fontBlob` (several times a build: `helper_geometry.rs`, `helper_font.rs`, and the readers listed below).
- Fix: the doc's Fix Design shape 1 (read in place): internal native readers over (font handle, offset) on the table's read path, with an unregistered font reading as "no font" as an empty blob does today; the builders use them and the whole-file copy goes from the per-build path. Phase 1's copy count is still taken first; if it shows the copies come from the compiler snapshotting a list argument, STOP and report (that is a different rule). Perf: runs alone; the measure is arena bytes, which load does not move.

Building the geometry record of one `canvas::Text` costs about 390 µs with a system
font, and nearly all of it is copying the font file. `canvas::fontBlobUnchecked` answers
an owned copy of the font's bytes, and the text builders ask for it on every build —
more than once. With Arial (773,236 bytes) a scene that rebuilds 1,000 text records a
frame spends 389 ms a frame doing so; with a 304-byte font the same scene spends 7.9 ms.

A text record is rebuilt whenever the item's hash changes: it moved, its string changed,
its paint changed. So this is the cost of every animated label, and it is silent — the
picture is right and nothing is raised.

**The single correct behavior a fix produces:** the time to build a `canvas::Text`
record does not depend on the size of the font file. Measured as: the particle-bench
text row below within 2x of the same row run with the 304-byte fixture font.

References:

- Found by plan-150-B Phase 1 (`planning/completed/plan-150-B-*.md`), measuring a
  `canvas::ParticleSystem` whose template is a `Text`: every particle is a text item
  with its own position, so every particle is one build.
- `.ai/canvas-threading.md` §12 ("Why the font rasteriser is hand-rolled") and
  `src/codegen/builtins/canvas/gen_font_table.rs` (the font table and its readers).

## Failing Reproduction

```
bash tools/particle-bench/bench.sh target/debug/mfb row 1000 TEMPLATE=text
FONT=/path/to/fixture.ttf bash tools/particle-bench/bench.sh target/debug/mfb row 1000 TEMPLATE=text
```

`fixture.ttf` is the twelve-glyph test font, `tests/common/mod.rs:fixture_truetype`
(the bytes are also `tests/canvas/rt_canvas_hit.rs:FIXTURE_TTF`).

Observed, 2026-10-13, Apple M2 Max, 1,000 particles each the two-glyph text `AB`, least
time per frame on Metal:

| Font | Bytes | Least per frame | Per text record |
|---|---|---|---|
| `/System/Library/Fonts/Supplemental/Arial.ttf` | 773,236 | 389,317 µs | about 389 µs |
| the fixture font | 304 | 7,893 µs | about 7.9 µs |

At 10,000 particles with Arial: 3,875,595 µs a frame. The glyph bitmaps are cached and
shared (`glyphs=` does not grow), so this is not rasterisation.

Expected: the two rows within 2x of each other.

It does not need a particle system. Any scene of `Text` items whose position changes
each frame is the same builds; the particle bench is simply the instrument at hand.

## Root Cause

`src/codegen/builtins/canvas/gen_font_table.rs:lower_font_blob_unchecked` ends in
`builder.copy_flat_block(&ParameterType::list_of(ParameterType::Byte), …)`: the font's
bytes are copied into the caller's arena on every call, because a `List OF Byte` an
MFBASIC function returns is an owned value. `__canvas_fontBlob` is the wrapper every
text path calls.

One build of a glyph run (`helper_geometry.rs:__canvas_textGlyphRun`) calls it directly,
and then calls `__canvas_shapeRun([font], t.text)` (`helper_font.rs`), which calls it
again for each font of the chain. `grep -n fontBlob src/codegen/builtins/canvas/*.rs`
lists the readers: `helper_geometry.rs` (four sites: the glyph run, the stroked run's
edges, the text header, the `--debug` reference), `helper_font.rs` (shaping),
`helper_autopaint.rs` (the layout box), `helper_hit.rs` (the worker's box),
`func_measure_text.rs` (`canvas::measureText`) and `helper_particles.rs` (a `Text`
emitter's box — once per system per frame, not per particle).

**Not yet confirmed:** how many copies one build makes. 389 µs is about five copies of
773 KB at memory speed, and the two calls above account for two. The rest may be the
same list handed to helpers in a form the compiler snapshots (`.ai/collections.md`,
bug-496: a module-function call in a later operand makes a list operand a copy).
Phase 1 counts them: `MFB_CANVAS_STATS`'s arena lines (`arena.1.alloc_bytes`) before and
after one text build, divided by the font's size.

## Non-goals

- **No change to what is drawn.** The text goldens (`rt_canvas_font`, the `transforms`
  and `group_affine` references) must not move.
- **No change to the font table's lifetime rules** (`.ai/canvas-threading.md` §13: a
  chain of fixed chunks, published name-last, readable by the graphics thread without a
  lock). A fix that hands MFBASIC a pointer into the table and reads it after
  `destroyFont` has unregistered the font is the tempting wrong fix: today's copy is what
  makes a build safe against a concurrent unregister.
- **Not** "use a smaller font in the benchmark". The benchmark is right; the cost is real.

## Blast Radius

| Site | Classification |
|---|---|
| `__canvas_textGlyphRun`, `__canvas_textEdges`, `__canvas_textHeader` (renderer builds) | fixed by this bug |
| `__canvas_shapeRun` (shaping, every text path) | fixed by this bug |
| `canvas::measureText` (`func_measure_text.rs`) | same pattern, on the worker; fixed by this bug if the fix is at `__canvas_fontBlob`, otherwise latent — say which |
| `helper_hit.rs:__canvas_textBoxHeader`, `helper_autopaint.rs:__canvas_autoTextBox` | same pattern, once per text per rebuilt hit list / record; follows the fix |
| `helper_particles.rs:__canvas_particleTextBox` | once per system per frame; unaffected in practice, follows the fix |
| Glyph rasterisation (`helper_glyph.rs`) | unaffected: cached per (font, size, glyph) |

## Fix Design

Two shapes, to be chosen in Phase 1 from the copy count:

1. **Read in place.** Give the text builders native readers that take the font handle
   and an offset (`canvas::fontU16(id, at)` and the like) instead of a `List OF Byte`,
   so nothing is copied. The readers must take the table's own read path
   (`emit_font_blob_read`'s load-acquire), and a font unregistered mid-build must read
   as "no font", as an empty blob does today.
2. **Copy once per thread per font.** Keep a per-thread copy of the last font's bytes
   beside its handle and a registration stamp, and hand the builders that. One copy per
   font per thread instead of several per build; the stamp is what notices a handle
   reused after `destroyFont`.

The first is the complete fix and touches every reader; the second is small and leaves
one copy per font. If Phase 1 finds the copies come from the compiler snapshotting a
list argument rather than from `fontBlobUnchecked` itself, the fix is at those call
sites and neither of the above.

## Phases

### Phase 1 — Count, and pin it with a failing test

- [ ] Count the font-sized allocations of one text build (`arena.1.alloc_bytes` in the
      `--debug` report, one frame with a `Text` against one without).
- [ ] Add a test to `tests/canvas/rt_canvas_font.rs`: build 200 distinct text records
      with a font padded to 1 MB (the fixture with a trailing unused table) and with the
      fixture itself; assert the padded run's graphics-arena `alloc_bytes` is within 2x
      of the unpadded run's. It fails today.

Acceptance: the count is recorded here and the test fails for the stated reason.
Commit: —

### Phase 2 — The fix

- [ ] Implement the shape Phase 1's count selects.
- [ ] `cargo test --test rt_canvas_font --test rt_canvas_golden --test rt_canvas_hit`
      → pass, goldens unmodified.

Acceptance: Phase 1's test passes, and the reproduction above is within 2x of the
fixture-font row.
Commit: —

### Phase 3 — Record the number

- [ ] Re-run `tools/particle-bench` `TEMPLATE=text` at 100 / 1,000 / 10,000 and replace
      the text rows in `mfb man canvas types` (`ParticleSystem`) and in
      `planning/completed/plan-150-B-*.md`'s table with the new ones.

Acceptance: the manual quotes the measured number.
Commit: —

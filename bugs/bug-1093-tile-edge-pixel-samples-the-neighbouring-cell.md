# bug-1093: a `Tile` whose edge falls part-way across a pixel draws that pixel from the neighbouring cell of its sheet

Last updated: 2026-10-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: (to add) tests/canvas/rt_canvas_tile.rs — a tile at a fractional destination, sheet rows either side of its source a different colour

A `canvas::Tile` selects a rectangle of an image (`srcPosition`, `srcSize`) and draws
it into a destination rectangle (`position`, `size`). When an edge of the destination
falls part-way across a pixel — any tile placed at a fractional position, such as a
sprite that bobs, or a map group moved by a non-integer camera — that edge pixel is
drawn with a texel from **outside the source rectangle**: the row or column of the
image just beyond it. In an atlas that is the next cell of the sheet, so a coin drawn
from a sprite sheet shows a thin line of whatever sprite sits above or below it in the
sheet. Every renderer does it (software and Metal measured; the Vulkan shader shares
the code).

It is silent and it is exactly what `Tile` exists to avoid: plan-173 replaced cutting
each cell into its own image (where the outside of the cell *was* transparent) with
selecting the cell from the atlas, and the spec promises that the part of the image
outside a tile's source is not drawn ("A sample outside the image is transparent,
without moving or stretching the part that is inside").

**The single correct behavior a fix produces:** every pixel a tile covers, fully or
partly, takes its colour from inside the tile's source rectangle — a partly covered
edge pixel takes the source's own edge texel, weighted by its coverage — and never from
a texel of the image outside that rectangle. Texels outside the *image* stay
transparent, as now.

References:

- `mfb spec app canvas`, "A tile selects an image rectangle".
- plan-173 (atlas tiles); its Corrections.
- Found while rewriting `examples/canvas-demo` (scene VI's coins bob by a fraction of a
  pixel; the demo now places them on whole pixels, citing this bug).

## Failing Reproduction

A 2×3 image: row 0 red, row 1 green, row 2 blue. Two tiles of row 1 (green), 40×20 —
one at a whole-pixel position, one at (10.5, 50.5):

```
IMPORT app
IMPORT canvas
IMPORT color
IMPORT os
IMPORT vector
IMPORT collections

FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT px AS List OF Byte = []
  FOR r = 0 TO 2
    FOR c = 0 TO 1
      IF r = 0 THEN px = collections::append(px, [toByte(255), toByte(0), toByte(0), toByte(255)])
      IF r = 1 THEN px = collections::append(px, [toByte(0), toByte(255), toByte(0), toByte(255)])
      IF r = 2 THEN px = collections::append(px, [toByte(0), toByte(0), toByte(255), toByte(255)])
    NEXT
  NEXT
  RES img AS canvas::Image = canvas::createImage(2, 3, px)
  LET whole AS canvas::DrawItem = canvas::Tile[image := img, srcPosition := at(0.0, 1.0), srcSize := at(2.0, 1.0), position := at(10.0, 10.0), size := at(40.0, 20.0)]
  LET frac AS canvas::DrawItem = canvas::Tile[image := img, srcPosition := at(0.0, 1.0), srcSize := at(2.0, 1.0), position := at(10.5, 50.5), size := at(40.0, 20.0)]
  canvas::present([whole, frac])
  os::sleep(1000)
END FUNC
```

`mfb build --debug`, then run with `MFB_HEADLESS=1 MFB_CANVAS_SYNC=1
MFB_CANVAS_DUMP=f.rgba` (and again with `MFB_CANVAS_GPU=1`), reading column x = 30 of the
900-pixel-wide dump:

| y | software | Metal | expected |
| --- | --- | --- | --- |
| 10, 29 (whole tile) | (0,255,0) | (0,255,0) | (0,255,0) |
| 50 (top edge, half covered) | (0,188,0) | (0,188,0) | (0,188,0) |
| 51 | (0,255,0) | (0,255,0) | (0,255,0) |
| **70 (bottom edge, half covered)** | **(0,0,188)** | **(0,0,188)** | (0,188,0) |

The bottom edge pixel is half-covered **blue**: row 2 of the image, outside the source.
(The top edge happens to land inside the source and is right; which edge bleeds depends
on where the pixel centre falls.) The whole-pixel tile is right on both renderers.

## Root Cause

Every renderer maps a pixel to a texel by `srcPosition + p * srcSize / size` at the
pixel's centre and rejects only texels outside the **image**:

- GPU (Vulkan): `src/codegen/runtime/canvas/shaders/mfb_canvas.frag:tileColour` —
  `u = tileTexel(sx * dw + (q.x - x0) * sw, dw)`, then `if (u < 0 || v < 0 || u >= iw ||
  v >= ih) return ivec4(0)`.
- Metal: the same mapping in `src/target/macos_aarch64/app/metal.rs` (the shader's
  `tileColour` twin).
- Software: `src/codegen/builtins/canvas/helper_items.rs` (the `isTile` sampling path,
  `picSX`/`picSY`/`picSW`/`picSH`).

A pixel the destination covers only partly has its centre *outside* the destination
rectangle, so `p` is outside `[0, size)` and the texel lands outside
`[srcPosition, srcPosition + srcSize)`. That texel is inside the image whenever the
source is not at the image's edge — in an atlas, almost always — so it is drawn,
weighted by the pixel's coverage. A `Picture` does not show this because its image is
the whole source (outside it is outside the image).

## Non-goals

- Texels outside the **image** stay transparent (plan-173: the inside part is not
  stretched to fill the rest).
- No change to nearest-pixel scaling, to whole-pixel tiles (already right), to
  `Picture`, or to the record layout / GPU block words.
- Not "fixed" by rounding positions in callers: that hides it for one program.

## Fix (phased, test-first)

1. **RED.** Add the reproduction above to `tests/canvas/rt_canvas_tile.rs` (software and
   the Metal row): the half-covered edge pixel must be the source's own colour at half
   coverage. Confirm it fails today.
2. **Fix.** In all three samplers, clamp the texel index to the source rectangle —
   `[floor(srcX), ceil(srcX + srcW) - 1]` and the same for Y — *before* the image-bounds
   test, so a source running past the image still yields transparent there. Keep the
   three formulas identical (GPU rows compare against the software oracle).
3. **Verify.** `cargo test --test rt_canvas_tile`, the GPU rows
   (`scripts/test-canvas-gpu-rows.sh` per `.ai/testing-gates.md`), then the full suite.
   Update the spec's tile paragraph to state the edge rule.

## Blast-radius audit

- `Picture` sampling: unaffected — its source is the whole image.
- Tiles at whole-pixel positions and sizes: unaffected (no partly covered pixel).
- Tiles under a group transform that rotates or scales: also affected (their edges are
  generally fractional); covered by the same clamp.

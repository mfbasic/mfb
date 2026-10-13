# bug-1090: under a huge magnification the zero-gradient fallback lights a stray column of pixels far from the shape

Last updated: 2026-10-12
Effort: unknown (the coverage rule's fallback, three renderers)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: none yet

Found during `bug-staging-23` (2026-10-12) by bs23-agent-f while proving bug-991, re-run by
the coordinator, and placed in a later phase.

Under a transform, coverage is `clamp(0.5 - d / g, 0, 1)`, `g` the length of the central
differences of the distance. Where `g` comes out at or under `1e-6` the renderers substitute
`g = 1`, so the pixel is lit wherever `d < 0.5` in the ITEM's units, however far that is on
the surface. On the line equidistant from two parts of a shape the differences cancel, and
under a large magnification that line is a visible column.

**The single correct behavior a fix produces:** the scene below lights no pixel (both squares
are off the surface), on software and on every GPU backend.

## Reproduction (main's compiler at `471aaff16`, software)

`bugs/repro/bug-1090/fallback_column.mfb`: one polygon of two small squares 0.9 units apart,
under an item transform of 14,000.

```
mfb build --app --debug <dir>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0 MFB_CANVAS_DUMP=sw.rgba <binary>
lit pixels: 191, all at x = 450, y 205..395
```

The agent measured the same 191 on Metal. `bugs/repro/bug-1083/fallback_column_in_group.mfb`
is the same item untransformed inside a group scaled 14,000.

## Relation to bug-991 / bug-1083

Their fix (landed 2026-10-12 by the owner's ruling) tightens a transformed item's box, which
cuts part of this column: 73 lit pixels instead of 191 for the item transform, 19 inside the
group. The owner accepted that change. The column that remains is this bug; the rim that
would keep all of it is `0.5 * max(F, 1)`, the old loose box.

## Where

`shapeCoverage` in `mfb_canvas.frag` and `metal.rs:METAL_SHADER_BODY`
(`if (!(g.x > 0.000001)) { g = vec2(1.0, 0.0); }`), and the oracle's twin in
`helper_items.rs:__canvas_drawGeometry`.

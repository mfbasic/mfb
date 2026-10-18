# bug-1103: a banded polygon drawn under a group matrix renders about 1.4x slower on Metal since bug-1085

Last updated: 2026-10-17
Effort: medium
Severity: LOW
Class: Other (Performance)

Status: Open
Regression Test: none yet; the instrument is the probe below

Found by the bug-staging-24 coordinator reviewing bug-1085, and placed in a later phase. The
owner accepted bug-1085's cost on 2026-10-17 and asked for the speed-up as its own bug.

bug-1085 made the band index exact under a group matrix: every band is built for at least
the identity's norm (`F_built = max(F, √2)`, reach `half + 0.5 + 2·F_built` + the 1-unit
guard), and a pixel whose composed norm the reach does not cover walks every edge. Main's
narrower untransformed reach (`half + 1.5`) was never covered by the five-sample argument
under a group; its frames matched by luck. bs24-agent-a measured that any reach the argument
allows costs at least about 1.25x on this scene (an uncommitted `+F_built` reach: about 490
ms). Getting back to main's speed needs a tighter bound than the Frobenius-norm argument
(for example the operator norm, which is 1 for a rotation), not a narrower reach.

**The single correct behavior a fix produces:** the probe renders within noise of main
`7781cd71b` with the GPU frames still matching the software oracle.

## Reproduction (Metal, M2 Max, load about 4, warm runs)

`/tmp/bs24-rev/bandrot/src/main.mfb` (kept below): 40 frames of 300 `canvas::Group`s, each
naming one group holding an untransformed 256-edge polygon of radius 25; `ROT=1` makes the
group transform a 30° rotation. `mfb build -q --app --debug .`, then
`ROT=… MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS=<file> <app>`, last line's
`phaseRenderMs`:

| compiler | identity group | rotated group |
|---|---|---|
| main `7781cd71b` | 382, 412 | 437, 416, 410 |
| bug-1085 fixed | 569 | 556 (bs24-agent-a: 570, 570 / 563, 557) |

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT collections
IMPORT color
IMPORT math
IMPORT os

FUNC pt(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC ring(n AS Integer, r AS Float) AS List OF vector::Float2
  MUT points AS List OF vector::Float2 = []
  MUT i AS Integer = 0
  WHILE i < n
    LET a AS Float = toFloat(i) * 6.283185307179586 / toFloat(n) + 0.2
    points = collections::append(points, pt(r * math::cos(a), r * math::sin(a)))
    i = i + 1
  END WHILE
  RETURN points
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET none AS canvas::Transform = canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
  canvas::setGroup("g", [canvas::Polygon[id := "", position := pt(0.0, 0.0), contours := [ring(256, 25.0)], rule := canvas::FillRule.NonZero, paint := canvas::fill(color::rgb(250, 200, 90)), transform := none]])
  MUT t AS canvas::Transform = canvas::Transform[a := 1.0, b := 0.0, c := 0.0, d := 1.0, tx := 0.0, ty := 0.0]
  IF os::getEnvOr("ROT", "0") = "1" THEN
    t = canvas::Transform[a := 0.866, b := 0.5, c := 0.0 - 0.5, d := 0.866, tx := 0.0, ty := 0.0]
  END IF
  MUT frame AS Integer = 0
  WHILE frame < 40
    MUT items AS List OF canvas::DrawItem = []
    MUT k AS Integer = 0
    WHILE k < 300
      LET x AS Float = 30.0 + toFloat(k MOD 20) * 43.0 + toFloat(frame MOD 2) * 0.25
      LET y AS Float = 30.0 + toFloat(k / 20) * 40.0
      items = collections::append(items, canvas::Group[id := "", position := pt(x, y), transform := t, name := "g"])
      k = k + 1
    END WHILE
    canvas::present(items)
    canvas::waitFrame()
    frame = frame + 1
  END WHILE
END FUNC
```

The first run of each build pays bug-1087's cold pipeline compile; discard it.

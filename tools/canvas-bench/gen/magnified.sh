#!/bin/bash
# gen/magnified.sh DIR MODE FRAMES
#
# bug-991's probe (`bugs/repro/bug-991/magnified.mfb` with the mode and frame count
# written in): 49 discs of 24 points, about 27 px across on the surface, drawn the same
# place in every mode. Only the coordinate space and the transform differ:
#   flat        radius 0.08 units, each item under `transform := scale(170)`
#   group       the same items untransformed, inside one `canvas::Group` scaled by 170
#   x10, x100   the items ten and a hundred times larger under a transform that much less
#   pix         the same discs in surface pixels, no transform -- the reference
#   circle-xf   a `Circle` of radius 0.08 under scale(170)
#   circle-pix  the same circle in pixels
# The fill's alpha changes every frame, so no frame is skipped. Run it with
# MFB_CANVAS_SYNC=1 so every present is a frame; the program ends after FRAMES.
set -euo pipefail
DIR=$1; MODE=$2; FRAMES=$3
case "$MODE" in flat|group|x10|x100|pix|circle-xf|circle-pix) ;; *) echo "magnified.sh: unknown mode $MODE" >&2; exit 1 ;; esac
mkdir -p "$DIR/src"
cat > "$DIR/project.json" <<J
{"name":"magnified","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
J
cat > "$DIR/src/main.mfb" <<M
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT math
IMPORT vector

FUNC tr(a AS Float, tx AS Float, ty AS Float) AS canvas::Transform
  RETURN canvas::Transform[a := a, b := 0.0, c := 0.0, d := a, tx := tx, ty := ty]
END FUNC

FUNC none() AS canvas::Transform
  RETURN canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
END FUNC

FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

' A 24-point disc of radius r about (ux, uy), scaled by k.
FUNC wedge(ux AS Float, uy AS Float, k AS Float, ox AS Float, oy AS Float) AS List OF vector::Float2
  MUT out AS List OF vector::Float2 = []
  FOR i = 0 TO 23
    LET a AS Float = toFloat(i) * 6.2831853 / 24.0
    out = collections::append(out, at(ox + (ux + math::cos(a) * 0.08) * k, oy + (uy + math::sin(a) * 0.08) * k))
  NEXT
  RETURN out
END FUNC

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  LET mode AS String = "$MODE"
  LET total AS Integer = $FRAMES
  LET R AS Float = 170.0
  MUT frame AS Integer = 0
  DO WHILE frame < total
    MUT scene AS List OF canvas::DrawItem = []
    FOR i = 0 TO 48
      LET ux AS Float = math::cos(toFloat(i) * 0.4) * (toFloat(i) / 60.0)
      LET uy AS Float = math::sin(toFloat(i) * 0.4) * (toFloat(i) / 60.0)
      LET k AS Integer = (frame + i) MOD 200
      LET p AS canvas::Paint = canvas::fill(color::rgba(200, 80, 80, 55 + k))
      MUT d AS canvas::DrawItem = canvas::Polygon[id := "", position := at(0.0, 0.0), contours := [wedge(ux, uy, 1.0, 0.0, 0.0)], rule := canvas::FillRule.NonZero, paint := p, transform := tr(R, 450.0, 300.0)]
      IF mode = "x10" THEN d = canvas::Polygon[id := "", position := at(0.0, 0.0), contours := [wedge(ux, uy, 10.0, 0.0, 0.0)], rule := canvas::FillRule.NonZero, paint := p, transform := tr(R / 10.0, 450.0, 300.0)]
      IF mode = "x100" THEN d = canvas::Polygon[id := "", position := at(0.0, 0.0), contours := [wedge(ux, uy, 100.0, 0.0, 0.0)], rule := canvas::FillRule.NonZero, paint := p, transform := tr(R / 100.0, 450.0, 300.0)]
      IF mode = "group" THEN d = canvas::Polygon[id := "", position := at(0.0, 0.0), contours := [wedge(ux, uy, 1.0, 0.0, 0.0)], rule := canvas::FillRule.NonZero, paint := p, transform := none()]
      IF mode = "pix" THEN d = canvas::Polygon[id := "", position := at(0.0, 0.0), contours := [wedge(ux, uy, R, 450.0, 300.0)], rule := canvas::FillRule.NonZero, paint := p, transform := none()]
      IF mode = "circle-pix" THEN d = canvas::Circle[id := "", position := at(450.0 + ux * R, 300.0 + uy * R), radius := 0.08 * R, paint := p, transform := none()]
      IF mode = "circle-xf" THEN d = canvas::Circle[id := "", position := at(450.0 + ux * R, 300.0 + uy * R), radius := 0.08, paint := p, transform := tr(R, 0.0, 0.0)]
      scene = collections::append(scene, d)
    NEXT
    IF mode = "group" THEN
      canvas::setGroup("g", scene)
      canvas::present([canvas::Group[id := "", position := at(0.0, 0.0), transform := tr(R, 450.0, 300.0), name := "g"]])
    ELSE
      canvas::present(scene)
    END IF
    frame = frame + 1
  LOOP
  RETURN 0
END FUNC
M

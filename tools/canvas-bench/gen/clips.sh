#!/bin/bash
# gen/clips.sh DIR ITEMS POINTS SCALE FRAMES MODE
#
# bug-995's probe: ITEMS squares, 100 px on the surface, tiled 15 across, each with a
# translucent fill and (POINTS > 0) a `Paint.clip` ring of POINTS points in its own
# coordinates. 32 points and up are banded (`BAND_MIN_EDGES`); fewer are walked whole.
#
# SCALE and MODE choose the path the clip takes through the shader:
#   SCALE=0            untransformed: one clip sample a pixel
#   MODE=item          every item carries `transform := scale(SCALE)` and is built in
#                      units of 1/SCALE, so it covers the same pixels
#   MODE=group         the items are untransformed, in units of 1/SCALE, inside one
#                      `canvas::Group` whose transform is the scale
# Both transformed modes take five clip samples a pixel wherever the centre sample
# cannot decide the pixel alone.
#
# The scene shifts a pixel every frame for FRAMES frames so no frame is skipped, and the
# program ends by itself. Run it with MFB_CANVAS_SYNC=1 so every present is a frame.
set -euo pipefail
DIR=$1; ITEMS=$2; POINTS=$3; SCALE=$4; FRAMES=$5; MODE=$6
case "$MODE" in item|group) ;; *) echo "clips.sh: MODE is item or group" >&2; exit 1 ;; esac
K=$SCALE
[ "$SCALE" = 0 ] && K=1.0
# A Float literal needs its point: `4` is an Integer.
case "$K" in *.*) ;; *) K="$K.0" ;; esac
mkdir -p "$DIR/src"
cat > "$DIR/project.json" <<J
{"name":"clips","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
J
ITEM_T="canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]"
SHIFT="toFloat(frame MOD 7)"
PRESENT="canvas::present(scene)"
if [ "$SCALE" != 0 ] && [ "$MODE" = item ]; then
  ITEM_T="canvas::Transform[a := $K, b := 0.0, c := 0.0, d := $K, tx := 0.0, ty := 0.0]"
fi
if [ "$SCALE" != 0 ] && [ "$MODE" = group ]; then
  # The group node moves instead of the items: the items are installed once a frame
  # all the same, so both modes pay the same worker cost.
  SHIFT="0.0"
  PRESENT="canvas::setGroup(\"tiles\", scene)
    canvas::present([canvas::Group[id := \"\", position := at(toFloat(frame MOD 7), 0.0), transform := canvas::Transform[a := $K, b := 0.0, c := 0.0, d := $K, tx := 0.0, ty := 0.0], name := \"tiles\"]])"
fi
cat > "$DIR/src/main.mfb" <<M
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT math
IMPORT vector

FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC ring(n AS Integer, cx AS Float, cy AS Float, r AS Float) AS List OF vector::Float2
  MUT out AS List OF vector::Float2 = []
  MUT i AS Integer = 0
  WHILE i < n
    LET a AS Float = toFloat(i) * 6.283185307179586 / toFloat(n)
    out = collections::append(out, at(cx + math::cos(a) * r, cy + math::sin(a) * r))
    i = i + 1
  END WHILE
  RETURN out
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET k AS Float = $K
  MUT frame AS Integer = 0
  WHILE frame < $FRAMES
    MUT scene AS List OF canvas::DrawItem = []
    MUT i AS Integer = 0
    WHILE i < $ITEMS
      LET x AS Float = (20.0 + toFloat(i MOD 15) * 52.0 + $SHIFT) / k
      LET y AS Float = (20.0 + toFloat((i / 15) MOD 10) * 52.0) / k
      LET w AS Float = 100.0 / k
      MUT cl AS List OF List OF vector::Float2 = []
      IF $POINTS > 0 THEN
        cl = [ring($POINTS, x + w / 2.0, y + w / 2.0, w * 0.45)]
      END IF
      scene = collections::append(scene, canvas::Polygon[id := "", position := at(0.0, 0.0), contours := [[at(x, y), at(x + w, y), at(x + w, y + w), at(x, y + w)]], rule := canvas::FillRule.NonZero, paint := WITH canvas::fill(color::rgba(80, 160, 220, 90)) { clip := cl }, transform := $ITEM_T])
      i = i + 1
    END WHILE
    $PRESENT
    frame = frame + 1
  END WHILE
END FUNC
M

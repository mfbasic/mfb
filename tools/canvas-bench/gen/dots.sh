#!/bin/bash
# gen/dots.sh DIR N SECONDS
#
# bug-765's scene: N fixed circles on a grid over the canvas, built twice with
# two fill colours, and the two scenes presented in turn for SECONDS. Nothing
# moves and nothing is added, so every frame changes every item's paint and no
# item's geometry. Both scenes are built before the clock starts.
#
# MOVE=1 moves every dot a little between the two scenes as well (the
# geometry-changing contrast); SAME=1 presents ONE scene every time (the
# identical-scene contrast, which the frame skip refuses, so it draws one
# frame).
#
# plan-175-A Phase 3: KIND picks what each dot is, and so which paint alternates:
#   circle    (the default) a filled circle; its fill colour;
#   gradient  a circle filled by a two-stop ramp; the ramp's colours;
#   picture   a 16 by 16 image in a square; the square's tint;
#   text      a two-letter `Text` in FONT; its colour.
# CONTROL=1 gives both scenes ONE paint and appends a small marker to scene B, so
# every frame still draws but every item's record is the last frame's: the
# contrast that leaves the rebuild (or refresh) of a paint change out.
#
# Prints, on stderr (a windowed app's stdout is the transcript):
#   count=N frames=<loop turns> pps=<presents/s> fps=<frames drawn/s> renderer=<Software|Metal|Vulkan> workerPresentUs=<us>
# pps, fps and renderer are canvas::getFrameStats's, read at the end of the run.
# workerPresentUs is the mean time inside one canvas::present call (the worker's
# deep copy, content compare, carried hashes, hashing and publish).
set -euo pipefail
DIR=$1; N=$2; SECS=$3
MOVE=${MOVE:-0}
SAME=${SAME:-0}
KIND=${KIND:-circle}
CONTROL=${CONTROL:-0}
FONT=${FONT:-/System/Library/Fonts/Supplemental/Arial.ttf}
FACE_DECL=""; FACE_ARG='""'
case "$KIND" in
  circle) DOT='RETURN canvas::Circle[id := "", position := vector::Float2[x := x, y := y], radius := radius, paint := canvas::fill(c), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]' ;;
  gradient) DOT='RETURN canvas::Circle[id := "", position := vector::Float2[x := x, y := y], radius := radius, paint := ramp(c, x - radius, x + radius), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]' ;;
  picture) DOT='RETURN canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := x - radius, y := y - radius], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := radius * 2.0, height := radius * 2.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(c), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]' ;;
  text)
    DOT='RETURN canvas::Text[id := "", position := vector::Float2[x := x - radius, y := y + radius], text := "Ag", fonts := [face], fontSize := radius * 2.0, letterSpacing := 0.0, paint := canvas::fill(c), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]'
    FACE_DECL="RES face AS canvas::Font = canvas::loadFont(\"$FONT\")" ;;
  *) echo "dots.sh: unknown KIND=$KIND" >&2; exit 2 ;;
esac
if [ "$KIND" = text ]; then FACE_PARAM=', face AS canvas::Font'; FACE_ARG=', face'; else FACE_PARAM=''; FACE_ARG=''; fi
# A grid of 8:5 cells that holds N: 400 by 250 at 100,000.
COLUMNS=$(awk -v n="$N" 'BEGIN{c=int(sqrt(n*1.6)); while (c*int(c*0.625) < n) c++; if (c < 1) c = 1; print c}')
mkdir -p "$DIR/src"
cat > "$DIR/project.json" <<J
{"name":"dots","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
J
cat > "$DIR/src/main.mfb" <<M
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT datetime
IMPORT io
IMPORT math
IMPORT vector

LET COLUMNS AS Integer = $COLUMNS
LET COUNT AS Integer = $N
LET ROWS AS Integer = (COUNT + COLUMNS - 1) / COLUMNS

FUNC pattern(w AS Integer, h AS Integer) AS List OF Byte
  MUT px AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < w * h
    px = collections::append(px, toByte(((i MOD w) * 13) MOD 256))
    px = collections::append(px, toByte(((i / w) * 11) MOD 256))
    px = collections::append(px, toByte(200))
    px = collections::append(px, toByte(255))
    i = i + 1
  END WHILE
  RETURN px
END FUNC

FUNC ramp(c AS color::Color, x0 AS Float, x1 AS Float) AS canvas::Paint
  MUT p AS canvas::Paint = canvas::fill(c)
  LET g AS canvas::Gradient = WITH p.gradient { startPoint := vector::Float2[x := x0, y := 0.0], endPoint := vector::Float2[x := x1, y := 0.0], stops := [canvas::GradientStop[offset := 0.0, color := c], canvas::GradientStop[offset := 1.0, color := color::rgb(255, 255, 255)]] }
  RETURN WITH p { gradient := g }
END FUNC

FUNC dot(x AS Float, y AS Float, radius AS Float, c AS color::Color, img AS canvas::Image$FACE_PARAM) AS canvas::DrawItem
  $DOT
END FUNC

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  app::setTitle("canvas-bench dots")
  RES img AS canvas::Image = canvas::createImage(16, 16, pattern(16, 16))
  $FACE_DECL
  LET size AS canvas::Size = canvas::getSize()
  LET cellW AS Float = size.width / toFloat(COLUMNS)
  LET cellH AS Float = size.height / toFloat(ROWS)
  LET radius AS Float = math::min(cellW, cellH) * 0.4
  LET empty AS canvas::DrawItem = dot(0.0, 0.0, radius, color::rgb(0, 0, 0), img$FACE_ARG)
  MUT sceneA AS List OF canvas::DrawItem = collections::repeat([], empty, COUNT)
  MUT sceneB AS List OF canvas::DrawItem = collections::repeat([], empty, COUNT)
  LET shift AS Float = toFloat($MOVE) * cellW * 0.1
  MUT inkB AS color::Color = color::rgb(192, 128, 63)
  IF $CONTROL = 1 THEN
    inkB = color::rgb(64, 128, 191)
  END IF
  FOR i = 0 TO COUNT - 1
    LET x AS Float = (toFloat(i MOD COLUMNS) + 0.5) * cellW
    LET y AS Float = (toFloat(i / COLUMNS) + 0.5) * cellH
    sceneA = collections::set(sceneA, i, dot(x, y, radius, color::rgb(64, 128, 191), img$FACE_ARG))
    sceneB = collections::set(sceneB, i, dot(x + shift, y, radius, inkB, img$FACE_ARG))
  NEXT
  IF $CONTROL = 1 THEN
    sceneB = collections::append(sceneB, canvas::Circle[id := "", position := vector::Float2[x := 4.0, y := 4.0], radius := 2.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]])
  END IF
  LET started AS Integer = datetime::monotonicNanos()
  MUT frame AS Integer = 0
  MUT presentNanos AS Integer = 0
  DO WHILE datetime::monotonicNanos() - started < $SECS * 1000000000
    LET before AS Integer = datetime::monotonicNanos()
    IF frame MOD 2 = 0 OR $SAME = 1 THEN
      canvas::present(sceneA)
    ELSE
      canvas::present(sceneB)
    END IF
    presentNanos = presentNanos + datetime::monotonicNanos() - before
    canvas::waitFrame()
    frame = frame + 1
  LOOP
  LET stats AS canvas::FrameStats = canvas::getFrameStats()
  MUT renderer AS String = "Software"
  IF stats.renderer = canvas::Renderer.Metal THEN
    renderer = "Metal"
  END IF
  IF stats.renderer = canvas::Renderer.Vulkan THEN
    renderer = "Vulkan"
  END IF
  MUT presentUs AS Integer = 0
  IF frame > 0 THEN
    presentUs = presentNanos / frame / 1000
  END IF
  io::printError("count=" & toString(COUNT) & " frames=" & toString(frame) & " pps=" & toString(stats.pps) & " fps=" & toString(stats.fps) & " renderer=" & renderer & " workerPresentUs=" & toString(presentUs))
  RETURN 0
END FUNC
M

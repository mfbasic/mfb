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
# Prints, on stderr (a windowed app's stdout is the transcript):
#   count=N frames=<loop turns> pps=<presents/s> fps=<frames drawn/s> renderer=<Software|Metal|Vulkan> workerPresentUs=<us>
# pps, fps and renderer are canvas::getFrameStats's, read at the end of the run.
# workerPresentUs is the mean time inside one canvas::present call (the worker's
# deep copy, content compare, carried hashes, hashing and publish).
set -euo pipefail
DIR=$1; N=$2; SECS=$3
MOVE=${MOVE:-0}
SAME=${SAME:-0}
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

FUNC dot(x AS Float, y AS Float, radius AS Float, c AS color::Color) AS canvas::DrawItem
  RETURN canvas::Circle[id := "", position := vector::Float2[x := x, y := y], radius := radius, paint := canvas::fill(c), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
END FUNC

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  app::setTitle("canvas-bench dots")
  LET size AS canvas::Size = canvas::getSize()
  LET cellW AS Float = size.width / toFloat(COLUMNS)
  LET cellH AS Float = size.height / toFloat(ROWS)
  LET radius AS Float = math::min(cellW, cellH) * 0.4
  LET empty AS canvas::DrawItem = dot(0.0, 0.0, radius, color::rgb(0, 0, 0))
  MUT sceneA AS List OF canvas::DrawItem = collections::repeat([], empty, COUNT)
  MUT sceneB AS List OF canvas::DrawItem = collections::repeat([], empty, COUNT)
  LET shift AS Float = toFloat($MOVE) * cellW * 0.1
  FOR i = 0 TO COUNT - 1
    LET x AS Float = (toFloat(i MOD COLUMNS) + 0.5) * cellW
    LET y AS Float = (toFloat(i / COLUMNS) + 0.5) * cellH
    sceneA = collections::set(sceneA, i, dot(x, y, radius, color::rgb(64, 128, 191)))
    sceneB = collections::set(sceneB, i, dot(x + shift, y, radius, color::rgb(192, 128, 63)))
  NEXT
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

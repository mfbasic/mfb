#!/bin/bash
# gen/pics.sh DIR TILES TILE_PX BG_W BG_H SECONDS ONESHOT(0|1)
#
# TILES pictures of two alternating TILE_PX x TILE_PX images in a grid,
# shifting every frame (a tilemap), plus an optional BG_W x BG_H background
# picture (0 0 = none). ONESHOT=1 presents one frame and exits — bug-686
# section H's picture/texel-cap probe.
set -euo pipefail
DIR=$1; N=$2; T=$3; BW=$4; BH=$5; SECS=$6; ONE=$7
mkdir -p "$DIR/src"
cat > "$DIR/project.json" <<J
{"name":"pics","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
J
cat > "$DIR/src/main.mfb" <<M
IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections
IMPORT datetime
IMPORT io

FUNC pattern(w AS Integer, h AS Integer, seed AS Integer) AS List OF Byte
  MUT px AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < w * h
    LET x AS Integer = i MOD w
    LET y AS Integer = i / w
    px = collections::append(px, toByte((x * 7 + seed) MOD 256))
    px = collections::append(px, toByte((y * 5 + seed * 3) MOD 256))
    px = collections::append(px, toByte(((x + y) * 3) MOD 256))
    px = collections::append(px, toByte(255))
    i = i + 1
  END WHILE
  RETURN px
END FUNC

SUB main()
  app::setMode(app::Mode.Canvas)
  RES tile AS canvas::Image = canvas::createImage($T, $T, pattern($T, $T, 1))
  RES tile2 AS canvas::Image = canvas::createImage($T, $T, pattern($T, $T, 90))
  RES bg AS canvas::Image = canvas::createImage(math_max1($BW), math_max1($BH), pattern(math_max1($BW), math_max1($BH), 40))
  LET white AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET start AS Integer = datetime::monotonicNanos()
  MUT frame AS Integer = 0
  MUT again AS Boolean = TRUE
  DO WHILE again
    MUT scene AS List OF canvas::DrawItem = []
    IF $BW > 0 THEN
      LET b AS canvas::DrawItem = canvas::Picture[x := 0.0, y := 0.0, w := 900.0, h := 640.0, image := bg, paint := white]
      scene = collections::append(scene, b)
    END IF
    MUT k AS Integer = 0
    WHILE k < $N
      LET x AS Float = toFloat((k * 11) MOD 880) + toFloat(frame MOD 5)
      LET y AS Float = toFloat((k * 7) MOD 620)
      ' Two distinct tile images alternate, as a real tilemap's would.
      IF k MOD 2 = 0 THEN
        LET p AS canvas::DrawItem = canvas::Picture[x := x, y := y, w := toFloat($T), h := toFloat($T), image := tile, paint := white]
        scene = collections::append(scene, p)
      ELSE
        LET p AS canvas::DrawItem = canvas::Picture[x := x, y := y, w := toFloat($T), h := toFloat($T), image := tile2, paint := white]
        scene = collections::append(scene, p)
      END IF
      k = k + 1
    END WHILE
    canvas::present(scene)
    frame = frame + 1
    IF $ONE = 1 THEN again = FALSE
    IF datetime::monotonicNanos() - start > $SECS * 1000000000 THEN again = FALSE
  LOOP
  io::print("presents=" & toString(frame))
END SUB

FUNC math_max1(v AS Integer) AS Integer
  IF v < 1 THEN RETURN 1
  RETURN v
END FUNC
M

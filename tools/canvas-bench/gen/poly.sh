#!/bin/bash
# gen/poly.sh DIR N -- one N-point polygon (a wavy ring, concave) plus a box.
# bug-686 section E/G's large-polygon probe: software vs Metal at one frame.
set -euo pipefail
DIR=$1; N=$2
mkdir -p "$DIR/src"
cat > "$DIR/project.json" <<J
{"name":"poly","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
J
cat > "$DIR/src/main.mfb" <<M
IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections
IMPORT math

SUB main()
  app::setMode(app::Mode.Canvas)
  MUT points AS List OF canvas::Point = []
  MUT i AS Integer = 0
  WHILE i < $N
    LET a AS Float = toFloat(i) * 6.283185307179586 / toFloat($N)
    LET r AS Float = 200.0 + 30.0 * math::sin(a * 7.0)
    points = collections::append(points, canvas::Point[x := 450.0 + r * math::cos(a), y := 320.0 + r * math::sin(a)])
    i = i + 1
  END WHILE
  LET ring AS canvas::DrawItem = canvas::Polygon[points := points, paint := canvas::fill(color::rgb(0, 200, 255))]
  LET box AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(color::rgb(0, 255, 0))]
  canvas::present([box, ring])
END SUB
M

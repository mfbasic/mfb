#!/bin/bash
# gen/coldstart.sh DIR [NAME] -- bug-996's cold-start probe: how long the first
# `canvas::present` of a process takes, and the second. NAME (default
# `coldstart`) is the project's, and so the executable's, name: a GPU driver's
# shader cache is per application, so a cold start needs a name it has not seen.
#
# One rectangle, presented twice (moved, so the second present is a frame).
# The program appends `firstPresentMs=… secondPresentMs=…` to the file named by
# its first argument rather than printing it, because in a real window there
# is no stdout to read. With MFB_CANVAS_SYNC set, `present` returns when its
# frame has been drawn, so the first number is the time to the first frame.
set -euo pipefail
DIR=$1; NAME=${2:-coldstart}
mkdir -p "$DIR/src"
cat > "$DIR/project.json" <<J
{"name":"$NAME","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
J
cat > "$DIR/src/main.mfb" <<M
IMPORT app
IMPORT canvas
IMPORT color
IMPORT datetime
IMPORT fs
IMPORT os
IMPORT collections
IMPORT vector

FUNC rect(x AS Float) AS canvas::DrawItem
  RETURN canvas::Rectangle[id := "", position := vector::Float2[x := x, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(200, 40, 40)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
END FUNC

FUNC main() AS Nothing
  LET start AS Integer = datetime::monotonicNanos()
  app::setMode(app::Mode.Canvas)
  canvas::present([rect(10.0)])
  LET first AS Integer = datetime::monotonicNanos()
  canvas::present([rect(20.0)])
  LET second AS Integer = datetime::monotonicNanos()
  fs::appendText(collections::get(os::args(), 0), "firstPresentMs=" & toString((first - start) / 1000000) & " secondPresentMs=" & toString((second - first) / 1000000) & "\n")
END FUNC
M

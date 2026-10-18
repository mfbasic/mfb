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
# `firstRenderer` is the first frame's renderer and `firstGpuFrameMs` the time
# from start to the first GPU-drawn frame (-1: none seen). With
# COLDSTART_WAIT_GPU set (bug-1087) it keeps presenting a moving rectangle until
# a frame is drawn by a GPU, for at most 60 s from start: a pipeline built off
# the frame path arrives some frames after the first.
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
  LET firstRenderer AS String = rendererName()
  canvas::present([rect(20.0)])
  LET second AS Integer = datetime::monotonicNanos()
  ' bug-1087: then present until a frame is drawn by a GPU, for at most 60 s, and say
  ' when: a pipeline built off the frame path arrives some frames later.
  MUT x AS Float = 30.0
  MUT gpuMs AS Integer = 0 - 1
  LET wait AS Boolean = len(os::getEnvOr("COLDSTART_WAIT_GPU", "")) > 0
  IF firstRenderer <> "software" THEN
    gpuMs = (first - start) / 1000000
  END IF
  WHILE wait AND gpuMs < 0 AND (datetime::monotonicNanos() - start) < 60000000000
    canvas::present([rect(x)])
    IF rendererName() <> "software" THEN
      gpuMs = (datetime::monotonicNanos() - start) / 1000000
    END IF
    x = x + 1.0
    IF x > 400.0 THEN
      x = 30.0
    END IF
  END WHILE
  fs::appendText(collections::get(os::args(), 0), "firstPresentMs=" & toString((first - start) / 1000000) & " secondPresentMs=" & toString((second - first) / 1000000) & " firstRenderer=" & firstRenderer & " firstGpuFrameMs=" & toString(gpuMs) & "\n")
END FUNC

FUNC rendererName() AS String
  LET s AS canvas::FrameStats = canvas::getFrameStats()
  IF s.renderer = canvas::Renderer.Metal THEN
    RETURN "metal"
  END IF
  IF s.renderer = canvas::Renderer.Vulkan THEN
    RETURN "vulkan"
  END IF
  RETURN "software"
END FUNC
M

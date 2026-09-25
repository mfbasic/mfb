#!/bin/bash
# gen/worker.sh DIR -- the worker-side microbenchmark from bug-686 section F:
# build, walk (FOR EACH + MATCH) and canvas::present a List OF DrawItem of
# 10,000 items, three repetitions (the first is cold). Release build only;
# it measures the worker thread's own cost, not the graphics thread's, so
# MFB_CANVAS_GPU=0 keeps the graphics thread out of the loop.
set -euo pipefail
DIR=$1
mkdir -p "$DIR/src"
cat > "$DIR/project.json" <<J
{"name":"mb","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
J
cat > "$DIR/src/main.mfb" <<M
IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections
IMPORT datetime
IMPORT io

FUNC ms(t AS Integer) AS String
  RETURN toString((datetime::monotonicNanos() - t) / 1000) & "us"
END FUNC

SUB main()
  app::setMode(app::Mode.Canvas)
  LET n AS Integer = 10000
  LET paint AS canvas::Paint = canvas::fill(color::rgba(0, 200, 255, 160))
  LET quad AS List OF canvas::Point = [canvas::Point[x := 0.0, y := 0.0], canvas::Point[x := 8.0, y := 0.0], canvas::Point[x := 8.0, y := 8.0], canvas::Point[x := 0.0, y := 8.0]]
  MUT rep AS Integer = 0
  WHILE rep < 3
    MUT t AS Integer = datetime::monotonicNanos()
    MUT rects AS List OF canvas::DrawItem = []
    MUT k AS Integer = 0
    WHILE k < n
      LET it AS canvas::DrawItem = canvas::Rectangle[x := toFloat(k MOD 880), y := toFloat(k MOD 620), w := 8.0, h := 8.0, paint := paint]
      rects = collections::append(rects, it)
      k = k + 1
    END WHILE
    io::print("build 10k rects: " & ms(t))
    t = datetime::monotonicNanos()
    MUT polys AS List OF canvas::DrawItem = []
    k = 0
    WHILE k < n
      LET it AS canvas::DrawItem = canvas::Polygon[points := quad, paint := paint]
      polys = collections::append(polys, it)
      k = k + 1
    END WHILE
    io::print("build 10k 4-pt polygons (shared points): " & ms(t))
    t = datetime::monotonicNanos()
    MUT pts AS List OF canvas::Point = []
    k = 0
    WHILE k < 40000
      pts = collections::append(pts, canvas::Point[x := toFloat(k MOD 880), y := 1.0])
      k = k + 1
    END WHILE
    io::print("build 40k points: " & ms(t))
    t = datetime::monotonicNanos()
    MUT sum AS Integer = 0
    FOR EACH item IN rects
      MATCH item
        CASE canvas::Rectangle(r) : sum = sum + 1
        CASE ELSE : sum = sum + 2
      END MATCH
    NEXT
    io::print("walk 10k items with MATCH: " & ms(t))
    t = datetime::monotonicNanos()
    canvas::present(rects)
    io::print("present 10k rects (first): " & ms(t))
    t = datetime::monotonicNanos()
    canvas::present(polys)
    io::print("present 10k polys: " & ms(t))
    rep = rep + 1
  END WHILE
END SUB
M

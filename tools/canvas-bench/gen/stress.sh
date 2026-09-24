#!/bin/bash
# gen/stress.sh DIR ITEMS POINTS MOVING(0|1) SECONDS
#
# ITEMS polygons of POINTS points (POINTS=4 -> quads), tiled across 900x640.
# MOVING=1: every polygon shifts each frame. MOVING=0: the bulk is static and
# only one small rect moves, since an unchanged scene is never re-rendered —
# that is bug-686 section B's "static" row.
#
# Optional env vars R=/CX=/CY= draw ITEMS as one big polygon of radius R
# centred at (CX,CY) when ITEMS=1 (section E's large-polygon probe).
#
# ONESHOT=1: present the static bulk once and exit (the `compare` oracle mode)
# instead of looping for SECONDS.
set -euo pipefail
DIR=$1; ITEMS=$2; POINTS=$3; MOVING=$4; SECS=$5
ONESHOT=${ONESHOT:-0}
mkdir -p "$DIR/src"
cat > "$DIR/project.json" <<J
{"name":"stress","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
J
cat > "$DIR/src/main.mfb" <<M
IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections
IMPORT datetime
IMPORT io
IMPORT math

FUNC ring(cx AS Float, cy AS Float, r AS Float) AS List OF canvas::Point
  MUT points AS List OF canvas::Point = []
  MUT i AS Integer = 0
  WHILE i < $POINTS
    LET a AS Float = toFloat(i) * 6.283185307179586 / toFloat($POINTS)
    points = collections::append(points, canvas::Point[x := cx + r * math::cos(a), y := cy + r * math::sin(a)])
    i = i + 1
  END WHILE
  RETURN points
END FUNC

FUNC bulk(shift AS Float) AS List OF canvas::DrawItem
  MUT items AS List OF canvas::DrawItem = []
  MUT k AS Integer = 0
  WHILE k < $ITEMS
    LET cx AS Float = toFloat((k * 37) MOD 880) + ${CX:-10.0} + shift
    LET cy AS Float = toFloat((k * 53) MOD 620) + ${CY:-10.0}
    LET it AS canvas::DrawItem = canvas::Polygon[points := ring(cx, cy, ${R:-8.0}), paint := canvas::fill(color::rgba(0, 200, 255, 160))]
    items = collections::append(items, it)
    k = k + 1
  END WHILE
  RETURN items
END FUNC

SUB main()
  app::setMode(app::Mode.Canvas)
  LET start AS Integer = datetime::monotonicNanos()
  LET still AS List OF canvas::DrawItem = bulk(0.0)
  MUT frame AS Integer = 0
  MUT buildNanos AS Integer = 0
  MUT presentNanos AS Integer = 0
  IF $ONESHOT = 1 THEN
    canvas::present(still)
    io::print("presents=1 workerBuildMs=0 workerPresentMs=0")
    EXIT SUB
  END IF
  DO WHILE datetime::monotonicNanos() - start < $SECS * 1000000000
    LET t0 AS Integer = datetime::monotonicNanos()
    MUT scene AS List OF canvas::DrawItem = still
    IF $MOVING = 1 THEN scene = bulk(toFloat(frame MOD 7))
    LET mover AS canvas::DrawItem = canvas::Rectangle[x := toFloat(frame MOD 800), y := 600.0, w := 20.0, h := 20.0, paint := canvas::fill(color::rgb(255, 0, 0))]
    scene = collections::append(scene, mover)
    buildNanos = buildNanos + (datetime::monotonicNanos() - t0)
    LET t1 AS Integer = datetime::monotonicNanos()
    canvas::present(scene)
    presentNanos = presentNanos + (datetime::monotonicNanos() - t1)
    frame = frame + 1
  LOOP
  io::print("presents=" & toString(frame) & " workerBuildMs=" & toString(buildNanos / 1000000) & " workerPresentMs=" & toString(presentNanos / 1000000))
END SUB
M

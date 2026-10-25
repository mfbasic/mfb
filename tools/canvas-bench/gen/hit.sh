#!/bin/bash
# gen/hit.sh DIR N IDS -- the hit-test microbenchmark (plan-172-I Phase 4): what
# `canvas::pollMouse` costs the worker to answer `MouseEvent.overID`.
#
# N 8x8 rectangles on a 10 px grid, 88 to a row. IDS=1 gives every one an `id`
# ("i0", "i1", ...); IDS=0 gives none, which is the scene every program written
# before `id` existed presents -- `pollMouse` must cost it nothing.
#
# 40 frames. Each one moves item 0, presents (so the scene really changed), then
# polls the sixty presses MFB_MOUSE_INJECT delivers:
#
#   poll-first  the first event after a present that changed the scene: the worker
#               rebuilds its draw list -- every item's geometry -- and walks it;
#   poll-again  each of the other fifty-nine, against the same scene: the walk
#               alone. Timed together and divided, because the clock ticks in
#               microseconds and an id-less poll is far under one.
#
# Every press is at pixel (444, 54), inside item 484. The walk runs from the LAST
# item back to it, so it is close to a whole-list walk: the dear case.
#
# `canvas::enableMouse(TRUE)` is what delivers the presses (the injected bytes go
# through the decoder there), so it brackets the polls and is outside the timing.
# An event not collected within a tenth of a second is dropped, so `events:` says
# how many of a frame's sixty the last frame actually got: fewer than 60 means the
# box was too slow for this size and poll-again is understated.
# Release build; MFB_CANVAS_GPU=0 keeps the graphics thread to software, and the
# program never waits on a frame.
#
# PAINT=1 (plan-175-A Phase 4): each frame recolours EVERY item instead of moving item
# 0, so the scene changes in paint alone. The worker's cache is keyed by the items'
# geometry identities, so poll-first then rebuilds nothing.
set -euo pipefail
DIR=$1; N=$2; IDS=$3; PAINT=${4:-0}
mkdir -p "$DIR/src"
cat > "$DIR/project.json" <<J
{"name":"hit","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
J
if [ "$IDS" = 1 ]; then NAME='"i" & toString(k)'; else NAME='""'; fi
cat > "$DIR/src/main.mfb" <<M
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT collections
IMPORT datetime
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET n AS Integer = $N
  LET paint AS canvas::Paint = canvas::fill(color::rgb(0, 200, 255))
  MUT first AS String = "poll-first-ns:"
  MUT again AS String = "poll-again-ns:"
  MUT presents AS String = "present-ns:"
  MUT over AS String = ""
  MUT events AS Integer = 0
  MUT frame AS Integer = 0
  WHILE frame < 40
    MUT items AS List OF canvas::DrawItem = []
    MUT k AS Integer = 0
    MUT ink AS canvas::Paint = paint
    IF $PAINT = 1 THEN
      ink = canvas::fill(color::rgb((frame * 6) MOD 256, 200, 255))
    END IF
    WHILE k < n
      MUT x AS Float = toFloat((k MOD 88) * 10)
      IF k = 0 AND $PAINT = 0 THEN
        x = x + toFloat(frame) / 100.0
      END IF
      LET it AS canvas::DrawItem = canvas::Rectangle[id := $NAME, position := vector::Float2[x := x, y := toFloat((k / 88) * 10)], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 8.0, height := 8.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
      items = collections::append(items, it)
      k = k + 1
    END WHILE
    MUT t AS Integer = datetime::monotonicNanos()
    canvas::present(items)
    presents = presents & " " & toString(datetime::monotonicNanos() - t)

    canvas::enableMouse(TRUE)
    t = datetime::monotonicNanos()
    LET a AS canvas::MouseEvent = canvas::pollMouse()
    first = first & " " & toString(datetime::monotonicNanos() - t)
    events = 0
    IF a.kind <> canvas::MouseKind.None THEN
      events = 1
      over = a.overID
    END IF
    t = datetime::monotonicNanos()
    MUT p AS Integer = 0
    WHILE p < 59
      LET b AS canvas::MouseEvent = canvas::pollMouse()
      IF b.kind <> canvas::MouseKind.None THEN
        events = events + 1
        over = b.overID
      END IF
      p = p + 1
    END WHILE
    again = again & " " & toString((datetime::monotonicNanos() - t) / 59)
    canvas::enableMouse(FALSE)
    frame = frame + 1
  END WHILE
  io::print(presents)
  io::print(first)
  io::print(again)
  io::print("over: [" & over & "]")
  io::print("events: " & toString(events))
END FUNC
M

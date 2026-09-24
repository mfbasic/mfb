#!/bin/bash
# gen/text.sh DIR LINES COLS SIZE SECONDS ONESHOT(0|1)
#
# LINES text rows of COLS characters at SIZE px, plus a moving cursor rect.
# bug-686 section H's text/quad/texel-cap probe.
set -euo pipefail
DIR=$1; L=$2; C=$3; SZ=$4; SECS=$5; ONE=$6
mkdir -p "$DIR/src"
cat > "$DIR/project.json" <<J
{"name":"txt","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
J
cat > "$DIR/src/main.mfb" <<M
IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections
IMPORT datetime
IMPORT io
IMPORT strings

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("/System/Library/Fonts/Supplemental/Arial.ttf")
  LET alphabet AS String = "The quick brown fox jumps over the lazy dog 0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz "
  MUT line AS String = ""
  WHILE strings::byteLen(line) < $C
    line = line & alphabet
  END WHILE
  line = strings::left(line, $C)
  MUT text AS List OF canvas::DrawItem = []
  MUT r AS Integer = 0
  WHILE r < $L
    LET t AS canvas::DrawItem = canvas::Text[x := 4.0, y := toFloat(r + 1) * toFloat($SZ) * 1.1, text := line, font := face, size := toFloat($SZ), paint := canvas::fill(color::rgb(230, 230, 230))]
    text = collections::append(text, t)
    r = r + 1
  END WHILE
  LET start AS Integer = datetime::monotonicNanos()
  MUT frame AS Integer = 0
  MUT again AS Boolean = TRUE
  DO WHILE again
    MUT scene AS List OF canvas::DrawItem = text
    LET cur AS canvas::DrawItem = canvas::Rectangle[x := toFloat(frame MOD 800), y := 600.0, w := 8.0, h := 16.0, paint := canvas::fill(color::rgb(255, 0, 0))]
    scene = collections::append(scene, cur)
    canvas::present(scene)
    frame = frame + 1
    IF $ONE = 1 THEN again = FALSE
    IF datetime::monotonicNanos() - start > $SECS * 1000000000 THEN again = FALSE
  LOOP
  io::print("presents=" & toString(frame))
END SUB
M

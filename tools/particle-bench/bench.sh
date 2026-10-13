#!/bin/bash
# tools/particle-bench/bench.sh -- plan-150-B's instrument: what a canvas::ParticleSystem
# costs a frame, on this machine, headless.
#
# Usage: bench.sh <mfb> sweep [FRAMES]
#        bench.sh <mfb> row COUNT [NAME=value ...]
#
#   sweep   the table plan-150-B asks for: 100 / 1,000 / 10,000 / 50,000 particles with
#           a circle template; the same counts with a rectangle template that spins (a
#           transformed particle), with a text template, with a polygon emitter, and
#           with canvas::pollMouse called every frame.
#   row     one row. NAME=value sets a knob of src/main.mfb: TEMPLATE, EMITTER, MOTION,
#           TINT, POLL, FRAMES.
#
# Every row is three runs of one program (src/main.mfb), built once normally and once
# with --debug:
#
#   gpu       the normal build under MFB_CANVAS_GPU=1: the least and mean microseconds
#             a present took, and the renderer that drew the last frame.
#   software  the normal build with no GPU asked for: the same two times.
#   stats     the --debug build under MFB_CANVAS_GPU=1 with MFB_CANVAS_STATS: frames
#             drawn and skipped, how many were GPU frames, the blocks of the last
#             frame, and the graphics thread's phase timers per frame (offsets = the
#             expansion and its geometry; draws = the draw list; render = the draw).
#
# A --debug build is slower than a normal one, so times are read from the normal build
# and only counts and the split between phases from the --debug one.
#
# MFB_CANVAS_SYNC=1 throughout: a present returns when its frame is drawn, so the time
# around it is a whole frame -- the worker's copy and hash, and the graphics thread's
# expansion, geometry and draw. Rank on the least (benchmark/RANKING.md section 1.1).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MFB="${1:?usage: bench.sh <mfb> sweep [FRAMES] | row COUNT [NAME=value ...]}"
MFB="$(cd "$(dirname "$MFB")" && pwd)/$(basename "$MFB")"
shift
CMD="${1:?sweep or row}"
shift

# A text template needs a real font. macOS ships this one; override with FONT=.
FONT="${FONT:-/System/Library/Fonts/Supplemental/Arial.ttf}"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/particle-bench.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

build() { # build <dir-name> [flag]
  local dir="$WORK/$1"
  mkdir -p "$dir"
  cp -R "$HERE/project.json" "$HERE/src" "$dir/"
  "$MFB" build -app ${2:-} "$dir" >"$dir/build.log" 2>&1 || {
    cat "$dir/build.log" >&2
    exit 1
  }
  echo "$dir/build/particle-bench.app/Contents/MacOS/particle-bench"
}

NORMAL="$(build normal)"
DEBUG="$(build debug --debug)"

field() { # field <line> <key>
  printf '%s\n' "$1" | tr ' ' '\n' | sed -n "s/^$2=//p" | tail -1
}

row() { # row COUNT [NAME=value ...]
  local count="$1"
  shift
  local knobs=("$@")
  local frames=120
  local k
  for k in ${knobs[@]+"${knobs[@]}"}; do
    case "$k" in FRAMES=*) frames="${k#FRAMES=}" ;; esac
  done
  local common=(MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 COUNT="$count" FRAMES="$frames" FONT="$FONT")
  local gpu sw
  gpu="$(env "${common[@]}" ${knobs[@]+"${knobs[@]}"} MFB_CANVAS_GPU=1 "$NORMAL" 2>/dev/null | tail -1)"
  sw="$(env "${common[@]}" ${knobs[@]+"${knobs[@]}"} "$NORMAL" 2>/dev/null | tail -1)"
  local stats="$WORK/stats.$$"
  rm -f "$stats"
  env "${common[@]}" ${knobs[@]+"${knobs[@]}"} MFB_CANVAS_GPU=1 MFB_CANVAS_STATS="$stats" "$DEBUG" >/dev/null 2>&1 || true
  local last="" drawn=0
  if [ -s "$stats" ]; then
    last="$(tail -1 "$stats")"
    drawn="$(wc -l <"$stats" | tr -d ' ')"
  fi
  local per
  per="$(awk -v n="$drawn" -v o="$(field "$last" phaseOffsetsMs)" -v d="$(field "$last" phaseDrawsMs)" -v r="$(field "$last" phaseRenderMs)" \
    'BEGIN { if (n > 0) printf "offsets=%.2f draws=%.2f render=%.2f", o / n, d / n, r / n; else printf "-" }')"
  printf 'particles/frame=%-6s %-34s | gpu: min=%sus mean=%sus renderer=%s | software: min=%sus mean=%sus renderer=%s | debug: frames=%s skipped=%s gpuFrames=%s blocks=%s phaseMs/frame: %s\n' \
    "$count" "${knobs[*]:-(circle, disc, moving, ramp)}" \
    "$(field "$gpu" minUs)" "$(field "$gpu" meanUs)" "$(field "$gpu" renderer)" \
    "$(field "$sw" minUs)" "$(field "$sw" meanUs)" "$(field "$sw" renderer)" \
    "$(field "$last" frames)" "$(field "$last" skipped)" "$(field "$last" gpuFrames)" "$(field "$last" blocks)" "$per"
}

case "$CMD" in
  row)
    row "$@"
    ;;
  sweep)
    FRAMES="${1:-120}"
    for count in 100 1000 10000 50000; do
      row "$count" FRAMES="$FRAMES"
    done
    for count in 100 1000 10000 50000; do
      row "$count" FRAMES="$FRAMES" TEMPLATE=rect MOTION=spinning
    done
    for count in 100 1000 10000; do
      row "$count" FRAMES="$FRAMES" TEMPLATE=text
    done
    for count in 100 1000 10000 50000; do
      row "$count" FRAMES="$FRAMES" EMITTER=polygon
    done
    for count in 100 1000 10000 50000; do
      row "$count" FRAMES="$FRAMES" POLL=1
    done
    ;;
  *)
    echo "unknown subcommand: $CMD" >&2
    exit 2
    ;;
esac

#!/usr/bin/env bash
# Build and run the canvas benchmark: one `--debug` build of benchmark/canvas/mfb, run
# once per row in a real window on macOS, every row's Metal frame profile reduced to one
# log block and its checksums. See benchmark/canvas/README.md for what each column is.
#
# Usage:
#   ./benchmark/canvas/run.sh                     # every row, 120 frames each
#   ./benchmark/canvas/run.sh --frames 240        # more frames per row
#   ./benchmark/canvas/run.sh --vsync off         # MFB_CANVAS_VSYNC=0 (see README)
#   ./benchmark/canvas/run.sh --only 'text|picture'   # rows whose name matches an ERE
#
# Each row opens a window. Keep the display awake and the windows unobstructed while it
# runs: a hidden or occluded window changes how the compositor paces its drawables.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"

frames=120
vsync=on
only=''
while [ $# -gt 0 ]; do
  case "$1" in
    --frames) frames="$2"; shift 2 ;;
    --vsync) vsync="$2"; shift 2 ;;
    --only) only="$2"; shift 2 ;;
    *) echo "usage: $0 [--frames N] [--vsync on|off] [--only ERE]" >&2; exit 2 ;;
  esac
done
case "$vsync" in on|off) ;; *) echo "error: --vsync takes on or off" >&2; exit 2 ;; esac
if [ "$(uname -s)" != Darwin ]; then
  echo "error: the canvas benchmark measures the Metal renderer, so it runs on macOS only" >&2
  exit 1
fi

MFB="${MFB:-$repo_root/target/debug/mfb}"
[ -x "$MFB" ] || MFB="$repo_root/target/release/mfb"
if [ ! -x "$MFB" ]; then
  echo "error: mfb binary not found (looked in target/debug and target/release)" >&2
  echo "build it first with: cargo build" >&2
  exit 1
fi
FONT="${FONT:-/System/Library/Fonts/Supplemental/Arial.ttf}"

# The rows: KIND PAINT COUNT-or-big. A kind with no inside (line, arc, polyline,
# polycurve) is always stroked, so its rows say "stroke".
rows=()
outlined=(line arc polyline polycurve)
filled=(rect rrect polygon circle ellipse text curvepolygon autopaint picture group tile)
for count in 1 100 10000; do
  for kind in "${filled[@]}"; do rows+=("$kind fill $count"); done
  for kind in "${outlined[@]}"; do rows+=("$kind stroke $count"); done
  rows+=("particles fill $count")
done
for count in 100 10000; do
  for kind in rect rrect polygon circle ellipse curvepolygon; do
    rows+=("$kind stroke $count" "$kind fillStroke $count")
  done
done
for kind in "${filled[@]}"; do rows+=("$kind fill big"); done
for kind in "${outlined[@]}"; do rows+=("$kind stroke big"); done

echo "==> building the canvas benchmark (--debug)"
"$MFB" build -app --debug "$here/mfb" >/dev/null
exe="$here/mfb/build/canvas-benchmark.app/Contents/MacOS/canvas-benchmark"

ts="$(date +%Y%m%d-%H%M%S)"
log="$here/canvas-$ts.log"
sums="$here/canvas-$ts.sums"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/canvas-bench.XXXXXX")"
trap 'rm -rf "$scratch" "$here/mfb/build"' EXIT

vsync_env=()
[ "$vsync" = off ] && vsync_env=(MFB_CANVAS_VSYNC=0)
{
  echo "# Lang: canvas (Metal, windowed, --debug build)"
  echo "# frames: $frames, the frames before the first Metal frame and that frame dropped; vsync: $vsync; display: $(system_profiler SPDisplaysDataType 2>/dev/null | sed -n 's/^ *Resolution: //p' | head -1)"
  echo "# columns: median, average, min, max (microseconds per frame)"
  echo
} >"$log"
: >"$sums"

# run_row KIND PAINT COUNT-or-big FRAMES STATS DUMP — one launch of the program. A row
# that hangs is killed after five minutes rather than holding the run.
run_row() {
  local kind="$1" paint="$2" size="$3" n="$4" stats="$5" dump="$6"
  local count=1 scale=small
  if [ "$size" = big ]; then scale=big; else count="$size"; fi
  local probe=()
  [ -n "$stats" ] && probe+=(MFB_CANVAS_STATS="$stats")
  [ -n "$dump" ] && probe+=(MFB_CANVAS_DUMP="$dump")
  env KIND="$kind" PAINT="$paint" COUNT="$count" SIZE="$scale" FRAMES="$n" FONT="$FONT" \
    MFB_CANVAS_SYNC=1 MFB_TRANSCRIPT_DUMP=1 ${vsync_env[@]+"${vsync_env[@]}"} "${probe[@]}" \
    perl -e 'alarm 300; exec @ARGV' "$exe" >/dev/null 2>&1
}

ran=0
for row in "${rows[@]}"; do
  if [ -n "$only" ] && ! [[ "$row" =~ $only ]]; then continue; fi
  read -r kind paint size <<<"$row"
  stats="$scratch/row.stats"
  dump="$scratch/row.rgba"
  rm -f "$stats" "$dump"
  printf '  %-31s' "$row"
  # The timed run, then a short run for the last frame's pixels: a frame that is dumped
  # is read back rather than presented, so the two cannot share a run.
  run_row "$kind" "$paint" "$size" "$frames" "$stats" ""
  run_row "$kind" "$paint" "$size" 2 "" "$dump"
  python3 "$here/reduce.py" "$row" "$stats" "$dump" "$vsync" "$log" "$sums"
  awk -v r="$row:" '$0 == r {s = 1; next} s && /^  work/ {print "work min " $5 " us"; exit}' "$log" | tr -d ','
  ran=$((ran + 1))
done
if [ "$ran" -eq 0 ]; then
  echo "error: no row matches --only '$only'" >&2
  exit 1
fi

echo
echo "==> $ran rows written (timestamp $ts):"
echo "    $log"
echo "    $sums"

#!/bin/bash
# tools/canvas-bench/bench.sh -- bug-686 canvas rendering performance instrument.
#
# Usage: bench.sh <mfb> <subcommand> [args...]
#
# Subcommands:
#   sweep [--release]
#   poly N [tag]
#   pics TILES TILE_PX BG_W BG_H
#   text LINES COLS SIZE
#   wind [window_secs]
#   worker
#   compare stress|poly|pics|text ARGS...
#
# See tools/canvas-bench/README.md for what each one probes and how to read
# the numbers. Everything runs headless (MFB_MACAPP_HEADLESS=1) with its
# projects under $TMPDIR, cleaned up on exit.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
GEN="$HERE/gen"
CMP="$HERE/cmp.py"

usage() {
  sed -n '2,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
  exit 1
}

[ $# -ge 2 ] || usage
MFB=$1; shift
CMD=$1; shift

WORK=$(mktemp -d "${TMPDIR:-/tmp}/canvas-bench.XXXXXX")
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

die() { echo "canvas-bench: $*" >&2; exit 1; }

# build_app DIR [--debug] -- `mfb build -app` (+ optional --debug), or die
# with the build's stderr.
build_app() {
  local dir=$1; local dbg=${2:-}
  if [ -n "$dbg" ]; then
    "$MFB" build -app --debug "$dir" >/dev/null 2>"$dir/build.err" || { cat "$dir/build.err" >&2; die "build failed: $dir"; }
  else
    "$MFB" build -app "$dir" >/dev/null 2>"$dir/build.err" || { cat "$dir/build.err" >&2; die "build failed: $dir"; }
  fi
}

# bin_path DIR NAME -- path to the headless app binary `mfb build -app` makes.
bin_path() { echo "$1/build/$2.app/Contents/MacOS/$2"; }

# stats_field FILE FIELD -- FIELD's value on FILE's last line, or empty.
stats_field() {
  tail -1 "$1" 2>/dev/null | grep -oE "(^| )$2=[0-9.]+" | tail -1 | cut -d= -f2
}

# stats_field_line LINE FIELD -- FIELD's value on a given line string.
stats_field_line() {
  echo "$1" | grep -oE "(^| )$2=[0-9.]+" | tail -1 | cut -d= -f2
}

# phase_fields FILE RENDERED -- per-frame ms for each phase timer the stats
# line carries (`spike*Ms=` / `phase*Ms=`, e.g. `phaseOffsetsMs`,
# `phaseDrawsMs`, `phaseDamageMs`, `phaseRenderMs`), or "-" if it carries
# none. Each field is a running cumulative total (like `generations`), so
# the per-frame figure is (last - first) / RENDERED, matching how
# `builds/frame` reads `generations`. With RENDERED <= 1 there is only one
# frame's worth of cumulative time, so the raw value on the last line already
# is the per-frame figure.
phase_fields() {
  local file=$1 rendered=$2
  local lastline firstline hits
  lastline=$(tail -1 "$file" 2>/dev/null || true)
  firstline=$(head -1 "$file" 2>/dev/null || true)
  hits=$(echo "$lastline" | grep -oE '(spike[A-Za-z]+|phase[A-Za-z]+)Ms=[0-9.]+' || true)
  if [ -z "$hits" ]; then echo "-"; return; fi
  local out=""
  while IFS='=' read -r name val; do
    [ -z "$name" ] && continue
    local fval; fval=$(echo "$firstline" | grep -oE "${name}=[0-9.]+" | cut -d= -f2)
    [ -z "$fval" ] && fval=0
    local perframe
    if [ "$rendered" -gt 1 ] 2>/dev/null; then
      perframe=$(awk -v a="$fval" -v b="$val" -v r="$rendered" 'BEGIN{printf "%.3f", (b-a)/r}')
    else
      perframe="$val"
    fi
    out="$out ${name}=${perframe}"
  done <<<"$hits"
  echo "$out" | sed 's/^ //'
}

# renderer_of RENDERED GPU_FRAMES -- both are cumulative totals read off the
# same (last) stats line, so "every rendered frame drew on Metal" is exact
# equality, not a delta over the run (the run's first line already has
# gpuFrames=1, not 0, so a delta is off by one).
renderer_of() {
  local rendered=$1 gpu=$2
  if [ "$rendered" -le 0 ]; then echo "n/a"; return; fi
  if [ "$gpu" -ge "$rendered" ]; then echo "Metal";
  elif [ "$gpu" -le 0 ]; then echo "software";
  else echo "mixed"; fi
}

# ---------------------------------------------------------------------------
# sweep
# ---------------------------------------------------------------------------
cmd_sweep() {
  local release=0
  [ "${1:-}" = "--release" ] && release=1
  local secs=5
  # items points moving  (label matches bug-686 section B's table order)
  local rows=(
    "95 8 1"
    "1000 8 1"
    "1000 8 0"
    "2000 8 1"
    "2500 8 1"
    "5000 3 1"
    "10000 4 1"
    "10 300 1"
    "100 300 1"
    "10 4000 1"
  )
  if [ "$release" = 1 ]; then
    echo "# release build, MFB_CANVAS_SYNC=1 (no --debug stats: renderer/geometry columns are n/a)"
  else
    echo "# --debug build (needed for MFB_CANVAS_STATS; ~35% slower on this path than release)"
  fi
  for row in "${rows[@]}"; do
    read -r items points moving <<<"$row"
    local dir="$WORK/sweep-$items-$points-$moving"
    bash "$GEN/stress.sh" "$dir" "$items" "$points" "$moving" "$secs"
    local fps renderer builds phases
    if [ "$release" = 1 ]; then
      build_app "$dir"
      local bin; bin=$(bin_path "$dir" stress)
      local out
      out=$(env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=1 MFB_CANVAS_SYNC=1 "$bin" 2>/dev/null | grep '^presents=')
      local presents; presents=$(stats_field_line "$out" presents)
      fps=$(awk -v p="$presents" -v s="$secs" 'BEGIN{printf "%.1f", p/s}')
      renderer="n/a (release, no stats)"
      builds="-"
      phases="-"
    else
      build_app "$dir" --debug
      local bin; bin=$(bin_path "$dir" stress)
      local stats="$dir/stats"
      env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS="$stats" "$bin" >/dev/null 2>&1 || true
      local rendered; rendered=$(wc -l < "$stats" | tr -d ' ')
      fps=$(awk -v r="$rendered" -v s="$secs" 'BEGIN{printf "%.1f", r/s}')
      local g0 g1; g0=$(stats_field_line "$(head -1 "$stats")" generations); g1=$(stats_field "$stats" generations)
      local gp1; gp1=$(stats_field "$stats" gpuFrames)
      renderer=$(renderer_of "$rendered" "$gp1")
      builds=$(awk -v a="$g0" -v b="$g1" -v r="$rendered" 'BEGIN{ if (r>0) printf "%.2f", (b-a)/r; else print "-" }')
      phases=$(phase_fields "$stats" "$rendered")
    fi
    printf "items=%-6s points=%-5s moving=%s | fps=%-6s renderer=%-24s builds/frame=%-6s phase: %s\n" \
      "$items" "$points" "$moving" "$fps" "$renderer" "$builds" "$phases"
  done
}

# ---------------------------------------------------------------------------
# poly
# ---------------------------------------------------------------------------
cmd_poly() {
  local n=${1:?"poly needs N"}; local tag=${2:-poly}
  local dir="$WORK/poly-$n"
  bash "$GEN/poly.sh" "$dir" "$n"
  build_app "$dir" --debug
  local bin; bin=$(bin_path "$dir" poly)
  for mode in sw gpu; do
    local gpu=0; [ "$mode" = gpu ] && gpu=1
    env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=$gpu \
      MFB_CANVAS_STATS="$dir/stats-$mode" MFB_CANVAS_DUMP="$dir/frame-$mode" "$bin" >/dev/null 2>&1
  done
  local cmp; cmp=$(python3 "$CMP" "$dir/frame-sw" "$dir/frame-gpu")
  local status; status=$(tail -1 "$dir/stats-gpu" 2>/dev/null | grep -oE '(gpuSelected|metalReady|gpuFrames)=[A-Za-z0-9]+' | tr '\n' ' ')
  echo "[$tag] N=$n $cmp | gpu: $status"
}

# ---------------------------------------------------------------------------
# pics
# ---------------------------------------------------------------------------
cmd_pics() {
  local tiles=${1:?"pics needs TILES"}; local tile_px=${2:?"pics needs TILE_PX"}
  local bw=${3:?"pics needs BG_W"}; local bh=${4:?"pics needs BG_H"}
  local live="$WORK/pics-live" oracle="$WORK/pics-oracle"
  bash "$GEN/pics.sh" "$live" "$tiles" "$tile_px" "$bw" "$bh" 5 0
  bash "$GEN/pics.sh" "$oracle" "$tiles" "$tile_px" "$bw" "$bh" 0 1
  build_app "$live" --debug
  build_app "$oracle" --debug
  local livebin; livebin=$(bin_path "$live" pics)
  env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS="$live/stats" "$livebin" >/dev/null 2>&1 || true
  local obin; obin=$(bin_path "$oracle" pics)
  for mode in sw gpu; do
    local gpu=0; [ "$mode" = gpu ] && gpu=1
    env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=$gpu \
      MFB_CANVAS_STATS="$oracle/stats-$mode" MFB_CANVAS_DUMP="$oracle/frame-$mode" "$obin" >/dev/null 2>&1
  done
  local texels=$((tiles * tile_px * tile_px + bw * bh))
  local rendered; rendered=$(wc -l < "$live/stats" | tr -d ' ')
  local live_status; live_status=$(tail -1 "$live/stats" | grep -oE 'gpuFrames=[0-9.]+' | tr '\n' ' ')
  local phases; phases=$(phase_fields "$live/stats" "$rendered")
  local cmp; cmp=$(python3 "$CMP" "$oracle/frame-sw" "$oracle/frame-gpu")
  local ostatus; ostatus=$(tail -1 "$oracle/stats-gpu" | grep -oE 'gpuFrames=[0-9]+')
  echo "tiles=$tiles of ${tile_px}px bg=${bw}x${bh} texels/frame=$texels | rendered=$rendered/5s $live_status phase: $phases | oracle: $cmp $ostatus"
}

# ---------------------------------------------------------------------------
# text
# ---------------------------------------------------------------------------
cmd_text() {
  local lines=${1:?"text needs LINES"}; local cols=${2:?"text needs COLS"}; local size=${3:?"text needs SIZE"}
  local live="$WORK/text-live" oracle="$WORK/text-oracle"
  bash "$GEN/text.sh" "$live" "$lines" "$cols" "$size" 5 0
  bash "$GEN/text.sh" "$oracle" "$lines" "$cols" "$size" 0 1
  build_app "$live" --debug
  build_app "$oracle" --debug
  local livebin; livebin=$(bin_path "$live" txt)
  env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS="$live/stats" "$livebin" >/dev/null 2>&1 || true
  local obin; obin=$(bin_path "$oracle" txt)
  for mode in sw gpu; do
    local gpu=0; [ "$mode" = gpu ] && gpu=1
    env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=$gpu \
      MFB_CANVAS_STATS="$oracle/stats-$mode" MFB_CANVAS_DUMP="$oracle/frame-$mode" "$obin" >/dev/null 2>&1
  done
  local rendered; rendered=$(wc -l < "$live/stats" | tr -d ' ')
  local live_status; live_status=$(tail -1 "$live/stats" | grep -oE '(gpuFrames|glyphs|glyphEvictions)=[0-9]+' | tr '\n' ' ')
  local phases; phases=$(phase_fields "$live/stats" "$rendered")
  local cmp; cmp=$(python3 "$CMP" "$oracle/frame-sw" "$oracle/frame-gpu")
  local ostatus; ostatus=$(tail -1 "$oracle/stats-gpu" | grep -oE 'gpuFrames=[0-9]+')
  echo "text ${lines}x${cols} chars at ${size}px | rendered=$rendered/5s $live_status phase: $phases | oracle: $cmp $ostatus"
}

# ---------------------------------------------------------------------------
# wind
# ---------------------------------------------------------------------------
cmd_wind() {
  local window=${1:-20}
  local dir="$WORK/wind"
  cp -R "$REPO_ROOT/examples/wind" "$dir"
  rm -rf "$dir/build"
  build_app "$dir" --debug
  local bin; bin=$(bin_path "$dir" wind)
  local stats="$dir/stats"
  ( env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS="$stats" "$bin" >/dev/null 2>&1 ) &
  local pid=$!
  local waited=0
  # examples/wind's fetch screen lasts up to ~30 s on a cold cache; give it
  # generous room (150 s, matching the spike) before giving up.
  until [ -f "$stats" ] && [ "$(wc -l < "$stats" | tr -d ' ')" -ge 2 ]; do
    sleep 1; waited=$((waited + 1))
    if [ "$waited" -ge 150 ]; then
      kill "$pid" 2>/dev/null || true
      die "wind: no second (first map) frame after 150s"
    fi
  done
  local a; a=$(wc -l < "$stats" | tr -d ' ')
  local la; la=$(sed -n "${a}p" "$stats")
  sleep "$window"
  local b; b=$(wc -l < "$stats" | tr -d ' ')
  local lb; lb=$(sed -n "${b}p" "$stats")
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  local frames=$((b - a))
  local fps; fps=$(awk -v f="$frames" -v w="$window" 'BEGIN{printf "%.2f", f/w}')
  echo "waited ${waited}s for first map frame; frames in ${window}s window = $frames (fps=$fps)"
  echo "  start: $(echo "$la" | grep -oE '(gpuFrames|blocks|spike[A-Za-z]+|phase[A-Za-z]+)=[0-9.]+' | tr '\n' ' ')"
  echo "  end:   $(echo "$lb" | grep -oE '(gpuFrames|blocks|generations|spike[A-Za-z]+|phase[A-Za-z]+)=[0-9.]+' | tr '\n' ' ')"
}

# ---------------------------------------------------------------------------
# worker
# ---------------------------------------------------------------------------
cmd_worker() {
  local dir="$WORK/worker"
  bash "$GEN/worker.sh" "$dir"
  build_app "$dir"
  local bin; bin=$(bin_path "$dir" mb)
  env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=0 "$bin"
}

# ---------------------------------------------------------------------------
# compare
# ---------------------------------------------------------------------------
compare_run() {
  # compare_run DIR BINNAME -- sw vs gpu dump on an already-generated project.
  local dir=$1 name=$2
  build_app "$dir" --debug
  local bin; bin=$(bin_path "$dir" "$name")
  for mode in sw gpu; do
    local gpu=0; [ "$mode" = gpu ] && gpu=1
    env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=$gpu \
      MFB_CANVAS_STATS="$dir/stats-$mode" MFB_CANVAS_DUMP="$dir/frame-$mode" "$bin" >/dev/null 2>&1
  done
  local cmp; cmp=$(python3 "$CMP" "$dir/frame-sw" "$dir/frame-gpu")
  local status; status=$(tail -1 "$dir/stats-gpu" 2>/dev/null | grep -oE '(gpuSelected|metalReady|gpuFrames)=[A-Za-z0-9]+' | tr '\n' ' ')
  echo "$cmp | gpu: $status"
}

cmd_compare() {
  local kind=${1:?"compare needs a scene kind: stress|poly|pics|text"}; shift
  case "$kind" in
    stress)
      local items=${1:?"compare stress needs ITEMS"}; local points=${2:?"compare stress needs POINTS"}
      local r=${3:-} cx=${4:-} cy=${5:-}
      # R/CX/CY plug into a Float parameter in the generated MFBASIC source, so
      # an integer-looking value (e.g. "300") needs a decimal point or the
      # compiler rejects it as an Integer/Float mismatch.
      case "$r" in *.*|"") ;; *) r="${r}.0" ;; esac
      case "$cx" in *.*|"") ;; *) cx="${cx}.0" ;; esac
      case "$cy" in *.*|"") ;; *) cy="${cy}.0" ;; esac
      local dir="$WORK/compare-stress"
      ( [ -n "$r" ] && export R="$r"; [ -n "$cx" ] && export CX="$cx"; [ -n "$cy" ] && export CY="$cy"
        ONESHOT=1 bash "$GEN/stress.sh" "$dir" "$items" "$points" 0 0 )
      echo "compare stress items=$items points=$points $( [ -n "$r" ] && echo "R=$r CX=$cx CY=$cy" )"
      compare_run "$dir" stress
      ;;
    poly)
      local n=${1:?"compare poly needs N"}
      local dir="$WORK/compare-poly"
      bash "$GEN/poly.sh" "$dir" "$n"
      echo "compare poly N=$n"
      compare_run "$dir" poly
      ;;
    pics)
      local tiles=${1:?"compare pics needs TILES"}; local tile_px=${2:?"needs TILE_PX"}
      local bw=${3:?"needs BG_W"}; local bh=${4:?"needs BG_H"}
      local dir="$WORK/compare-pics"
      bash "$GEN/pics.sh" "$dir" "$tiles" "$tile_px" "$bw" "$bh" 0 1
      echo "compare pics tiles=$tiles tile_px=$tile_px bg=${bw}x${bh}"
      compare_run "$dir" pics
      ;;
    text)
      local lines=${1:?"compare text needs LINES"}; local cols=${2:?"needs COLS"}; local size=${3:?"needs SIZE"}
      local dir="$WORK/compare-text"
      bash "$GEN/text.sh" "$dir" "$lines" "$cols" "$size" 0 1
      echo "compare text lines=$lines cols=$cols size=$size"
      compare_run "$dir" txt
      ;;
    *) die "compare: unknown kind '$kind' (want stress|poly|pics|text)" ;;
  esac
}

case "$CMD" in
  sweep) cmd_sweep "$@" ;;
  poly) cmd_poly "$@" ;;
  pics) cmd_pics "$@" ;;
  text) cmd_text "$@" ;;
  wind) cmd_wind "$@" ;;
  worker) cmd_worker "$@" ;;
  compare) cmd_compare "$@" ;;
  *) usage ;;
esac

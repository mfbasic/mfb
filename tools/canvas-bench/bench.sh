#!/bin/bash
# tools/canvas-bench/bench.sh -- bug-686 canvas rendering performance instrument
# (bug-688 adds a Linux/Vulkan runner alongside the original macOS/Metal one).
#
# Usage: bench.sh <mfb> [--target linux-aarch64|linux-x86_64 --box <ssh-port>
#                        [--libc glibc|musl]] <subcommand> [args...]
#        bench.sh <mfb> --target windows-x86_64 --host <user@host> <subcommand> [args...]
#
# Subcommands:
#   sweep [--release]
#   poly N [tag]
#   pics TILES TILE_PX BG_W BG_H
#   text LINES COLS SIZE
#   weather [window_secs]
#   worker
#   hit
#   clips [ITEMS] [FRAMES]
#   compare stress|poly|pics|text ARGS...
#
# Without --target/--box, everything runs headless (MFB_MACAPP_HEADLESS=1)
# and *locally* as a macOS `.app` bundle (`mfb build -app`), exactly as
# bug-686 left it.
#
# With --target/--box (bug-688), the same scenes build as a Linux AppImage
# (`mfb build --app --target <target>`) and run headless
# (MFB_GTKAPP_HEADLESS=1) on a remote box reached over
# `ssh -p <ssh-port> test@127.0.0.1`: shipped with `scp`, extracted with
# `--appimage-extract`, run under `timeout`, and its stats/dump files copied
# back so the rest of this script (cmp.py, the stats parsers below) reads
# them exactly as it reads a local macOS run. The Linux flags may appear
# anywhere in the argument list (before or after <mfb>/<subcommand>/its
# args) -- they are stripped out before positional parsing, so both
# `bench.sh <mfb> --target linux-aarch64 --box 2226 poly 1000` and
# `bench.sh <mfb> poly --target linux-aarch64 --box 2226 1000` work.
#
# With --target windows-x86_64 --host <user@host> (bug-688 V5), the scenes
# build as a Windows .exe and run headless (MFB_WINAPP_HEADLESS=1) on that
# host over key-auth ssh, through a CRLF .bat that sets the env vars (cmd's
# `set X=1 && prog` is unreliable over ssh); stats/dump files are copied back
# the same way. Box 206 (192.168.1.206) is a REAL GPU (Intel UHD 630), so its
# fps numbers are GPU numbers.
#
# WARNING: the Linux Vulkan boxes reachable from here run Mesa's lavapipe (a CPU
# rasteriser), not a GPU. The renderer/correctness columns (Vulkan vs.
# software, pixel-compare) are meaningful; the fps numbers are not -- they
# measure a CPU rasteriser's throughput, not a GPU's.
#
# See tools/canvas-bench/README.md for what each subcommand probes and how
# to read the numbers. Everything runs headless with its projects under
# $TMPDIR, cleaned up on exit (and, in Linux mode, under a
# /tmp/canvas-bench-* dir on the box, also cleaned up on exit).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
GEN="$HERE/gen"
CMP="$HERE/cmp.py"

# The font gen/text.sh's generated scene loads on a Linux box, since
# /System/Library/Fonts/Supplemental/Arial.ttf (the macOS default it bakes
# in) does not exist there. Present on the Debian box this was verified
# against (box 2226) under /usr/share/fonts/truetype/liberation2/ -- a
# metric-compatible Arial substitute, no font needs shipping.
LINUX_FONT="/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf"
WIN_FONT='C:\Windows\Fonts\arial.ttf'

usage() {
  sed -n '2,33p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
  exit 1
}

# ---------------------------------------------------------------------------
# Linux flags: --target/--box/--libc may appear anywhere in argv. Strip them
# out first, leaving <mfb> <subcommand> [args...] as plain positionals --
# same shape the macOS-only script always took.
# ---------------------------------------------------------------------------
LINUX_TARGET=""
LINUX_BOX=""
LINUX_LIBC="glibc"
REMOTE_HOST="test@127.0.0.1"
REMOTE_BASE=""
WIN_HOST=""

ARGV=()
while [ $# -gt 0 ]; do
  case "$1" in
    --target) LINUX_TARGET=${2:?"--target needs linux-aarch64|linux-x86_64"}; shift 2 ;;
    --box) LINUX_BOX=${2:?"--box needs an ssh port"}; shift 2 ;;
    --libc) LINUX_LIBC=${2:?"--libc needs glibc|musl"}; shift 2 ;;
    --host) WIN_HOST=${2:?"--host needs user@host"}; shift 2 ;;
    *) ARGV+=("$1"); shift ;;
  esac
done
set -- "${ARGV[@]+"${ARGV[@]}"}"

[ $# -ge 2 ] || usage
MFB=$1; shift
CMD=$1; shift

# WIN=1: --target windows-x86_64 --host <user@host>. It rides the Linux-mode
# code paths ($LINUX_TARGET set), with the remote helpers below branching on it.
WIN=0
if [ "$LINUX_TARGET" = "windows-x86_64" ]; then
  WIN=1
  [ -n "$WIN_HOST" ] || { echo "canvas-bench: --target windows-x86_64 requires --host <user@host>" >&2; exit 1; }
  LINUX_FONT="$WIN_FONT"
elif [ -n "$LINUX_TARGET" ] || [ -n "$LINUX_BOX" ]; then
  [ -n "$LINUX_TARGET" ] || { echo "canvas-bench: --box requires --target linux-aarch64|linux-x86_64" >&2; exit 1; }
  [ -n "$LINUX_BOX" ] || { echo "canvas-bench: --target requires --box <ssh-port>" >&2; exit 1; }
fi

WORK=$(mktemp -d "${TMPDIR:-/tmp}/canvas-bench.XXXXXX")
if [ "$WIN" = 1 ]; then
  # Relative to the Windows user's home (scp and a fresh ssh both start there).
  REMOTE_BASE="canvas-bench-$(basename "$WORK" | tr -c 'A-Za-z0-9\n' '_')"
  cleanup() {
    rm -rf "$WORK"
    ssh -o BatchMode=yes -o ConnectTimeout=10 "$WIN_HOST" "rmdir /s /q $REMOTE_BASE" </dev/null >/dev/null 2>&1 || true
  }
elif [ -n "$LINUX_TARGET" ]; then
  REMOTE_BASE="/tmp/canvas-bench-$(basename "$WORK")"
  cleanup() {
    rm -rf "$WORK"
    ssh -o BatchMode=yes -o ConnectTimeout=10 -p "$LINUX_BOX" "$REMOTE_HOST" "rm -rf '$REMOTE_BASE'" </dev/null >/dev/null 2>&1 || true
  }
else
  cleanup() { rm -rf "$WORK"; }
fi
trap cleanup EXIT

die() { echo "canvas-bench: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Linux remote helpers (only ever called when $LINUX_TARGET is set).
# ---------------------------------------------------------------------------

# ssh_box CMD -- run CMD on the box; </dev/null so a caller inside a `while
# read`/`until` loop never has its stdin swallowed by ssh.
ssh_box() {
  if [ "$WIN" = 1 ]; then
    ssh -o BatchMode=yes -o ConnectTimeout=10 "$WIN_HOST" "$1" </dev/null
  else
    ssh -o BatchMode=yes -o ConnectTimeout=10 -p "$LINUX_BOX" "$REMOTE_HOST" "$1" </dev/null
  fi
}

scp_to_box() {
  if [ "$WIN" = 1 ]; then scp -o BatchMode=yes -o ConnectTimeout=10 -q "$1" "$WIN_HOST:$2"
  else scp -P "$LINUX_BOX" -o BatchMode=yes -o ConnectTimeout=10 -q "$1" "$REMOTE_HOST:$2"; fi
}
scp_from_box() {
  if [ "$WIN" = 1 ]; then scp -o BatchMode=yes -o ConnectTimeout=10 -q "$WIN_HOST:$1" "$2"
  else scp -P "$LINUX_BOX" -o BatchMode=yes -o ConnectTimeout=10 -q "$REMOTE_HOST:$1" "$2"; fi
}

# win_path REL -- a home-relative forward-slash path as a cmd.exe path.
win_path() { echo "%USERPROFILE%\\$(echo "$1" | tr '/' '\\')"; }

# project_name DIR -- the "name" field out of a gen/*.sh project.json (the
# AppImage and its extracted binary are named after it).
project_name() {
  sed -n 's/.*"name": *"\([^"]*\)".*/\1/p' "$1/project.json" | head -1
}

# ship_app DIR NAME -- scp DIR's built AppImage to a fresh remote dir under
# $REMOTE_BASE and `--appimage-extract` it there; idempotent per DIR (a
# marker file records the remote dir once shipped). Dies if the AppImage or
# its extracted binary is missing.
ship_app() {
  local dir=$1 name=$2
  local marker="$dir/.linux-remote-dir"
  [ -f "$marker" ] && return 0
  local rname; rname=$(basename "$dir")
  local rdir="$REMOTE_BASE/$rname"
  if [ "$WIN" = 1 ]; then
    local exe="$dir/build/${name}.exe"
    [ -f "$exe" ] || die "windows build missing artifact: $exe"
    ssh_box "mkdir $(win_path "$rdir")" >/dev/null 2>&1 || true
    scp_to_box "$exe" "$rdir/$name.exe"
    echo "$rdir" > "$marker"
    return 0
  fi
  local appimg="$dir/build/${name}-${LINUX_LIBC}.AppImage"
  [ -f "$appimg" ] || die "linux build missing artifact: $appimg"
  ssh_box "mkdir -p '$rdir'"
  scp_to_box "$appimg" "$rdir/app.AppImage"
  ssh_box "cd '$rdir' && chmod +x app.AppImage && ./app.AppImage --appimage-extract >extract.log 2>&1"
  ssh_box "test -x '$rdir/squashfs-root/usr/bin/$name'" || die "linux extract missing binary: $rdir/squashfs-root/usr/bin/$name"
  echo "$rdir" > "$marker"
}

remote_dir_of() {
  cat "$1/.linux-remote-dir" 2>/dev/null || die "no shipped app for $1 (build_app not called?)"
}

# linux_run DIR NAME OUTFILE ENV_ASSIGN... -- run DIR's shipped NAME binary
# on the box with MFB_GTKAPP_HEADLESS=1 plus ENV_ASSIGN (NAME=VALUE strings,
# same shape as the macOS call sites' `env` args), time-boxed to 120s.
# OUTFILE ("" to discard) gets the run's combined stdout+stderr. Any
# ENV_ASSIGN value that is a path under DIR (MFB_CANVAS_STATS,
# MFB_CANVAS_DUMP) is an output file the program writes: it is remapped to a
# path under the box's remote dir and scp'd back to that same local path
# after the run, so callers read $dir/stats, $dir/frame-gpu, etc. exactly as
# they do on macOS. Does not itself raise on a nonzero exit -- same as most
# macOS call sites, which run under `set -e` unguarded and let a real
# failure abort the script; callers that want `|| true` add it themselves.
linux_run() {
  local dir=$1 name=$2 outfile=$3; shift 3
  local rdir; rdir=$(remote_dir_of "$dir")
  local remote_envs=()
  local fetch_remote=() fetch_local=()
  local a k v base rpath
  for a in "$@"; do
    k=${a%%=*}; v=${a#*=}
    case "$v" in
      "$dir"/*)
        base=$(basename "$v")
        rpath="$rdir/$base"
        remote_envs+=("$k=$rpath")
        fetch_remote+=("$rpath")
        fetch_local+=("$v")
        ;;
      *) remote_envs+=("$a") ;;
    esac
  done
  if [ "$WIN" = 1 ]; then
    # A CRLF .bat sets the env vars (paths remapped to the remote dir, in cmd
    # form) and runs the exe; the program ends itself (every gen/ scene is
    # time-boxed), so no external timeout is needed.
    local bat="$dir/run.bat" e i
    {
      printf 'cd /d %s\r\n' "$(win_path "$rdir")"
      printf 'set MFB_WINAPP_HEADLESS=1\r\n'
      for e in ${remote_envs[@]+"${remote_envs[@]}"}; do
        local ek=${e%%=*} ev=${e#*=}
        case "$ev" in "$rdir"/*) ev=$(win_path "$ev") ;; esac
        printf 'set %s=%s\r\n' "$ek" "$ev"
      done
      printf '%s.exe\r\n' "$name"
    } > "$bat"
    scp_to_box "$bat" "$rdir/run.bat"
    local wcmd; wcmd="$(win_path "$rdir")\\run.bat"
    if [ -n "$outfile" ]; then
      ssh_box "$wcmd" >"$outfile" 2>&1 || true
    else
      ssh_box "$wcmd" >/dev/null 2>&1 || true
    fi
    for ((i = 0; i < ${#fetch_remote[@]}; i++)); do
      scp_from_box "${fetch_remote[$i]}" "${fetch_local[$i]}" 2>/dev/null || true
    done
    return 0
  fi
  local remote_cmd="cd '$rdir' && timeout 120 env MFB_GTKAPP_HEADLESS=1 ${remote_envs[*]+"${remote_envs[*]}"} ./squashfs-root/usr/bin/$name"
  if [ -n "$outfile" ]; then
    ssh_box "$remote_cmd" >"$outfile" 2>&1
  else
    ssh_box "$remote_cmd" >/dev/null 2>&1
  fi
  local i
  for ((i = 0; i < ${#fetch_remote[@]}; i++)); do
    scp_from_box "${fetch_remote[$i]}" "${fetch_local[$i]}" 2>/dev/null || true
  done
}

# build_app DIR [--debug] -- `mfb build -app` (+ optional --debug) locally
# for macOS, or `mfb build --app --target $LINUX_TARGET` (+ optional
# --debug) plus shipping the result to the box for Linux, or die with the
# build's stderr.
build_app() {
  local dir=$1; local dbg=${2:-}
  if [ -n "$LINUX_TARGET" ]; then
    if [ -n "$dbg" ]; then
      "$MFB" build --app --debug --target "$LINUX_TARGET" "$dir" >/dev/null 2>"$dir/build.err" || { cat "$dir/build.err" >&2; die "build failed: $dir"; }
    else
      "$MFB" build --app --target "$LINUX_TARGET" "$dir" >/dev/null 2>"$dir/build.err" || { cat "$dir/build.err" >&2; die "build failed: $dir"; }
    fi
    ship_app "$dir" "$(project_name "$dir")"
  else
    if [ -n "$dbg" ]; then
      "$MFB" build -app --debug "$dir" >/dev/null 2>"$dir/build.err" || { cat "$dir/build.err" >&2; die "build failed: $dir"; }
    else
      "$MFB" build -app "$dir" >/dev/null 2>"$dir/build.err" || { cat "$dir/build.err" >&2; die "build failed: $dir"; }
    fi
  fi
}

# bin_path DIR NAME -- path to the headless app binary `mfb build -app` makes
# (macOS only; Linux mode reaches its binary through ship_app/linux_run).
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
# same (last) stats line, so "every rendered frame drew on the GPU backend"
# is exact equality, not a delta over the run (the run's first line already
# has gpuFrames=1, not 0, so a delta is off by one). Reports "Metal" in
# macOS mode, "Vulkan" in Linux mode (the two backends this script drives).
renderer_of() {
  local rendered=$1 gpu=$2
  local label="Metal"
  [ -n "$LINUX_TARGET" ] && label="Vulkan"
  if [ "$rendered" -le 0 ]; then echo "n/a"; return; fi
  if [ "$gpu" -ge "$rendered" ]; then echo "$label";
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
      if [ -n "$LINUX_TARGET" ]; then
        build_app "$dir"
        local outfile="$dir/run.out"
        linux_run "$dir" stress "$outfile" "MFB_CANVAS_GPU=1" "MFB_CANVAS_SYNC=1"
        local out; out=$(grep '^presents=' "$outfile" || true)
        local presents; presents=$(stats_field_line "$out" presents)
        fps=$(awk -v p="$presents" -v s="$secs" 'BEGIN{printf "%.1f", p/s}')
        renderer="n/a (release, no stats)"
        builds="-"
        phases="-"
      else
        build_app "$dir"
        local bin; bin=$(bin_path "$dir" stress)
        local out
        out=$(env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=1 MFB_CANVAS_SYNC=1 "$bin" 2>/dev/null | grep '^presents=')
        local presents; presents=$(stats_field_line "$out" presents)
        fps=$(awk -v p="$presents" -v s="$secs" 'BEGIN{printf "%.1f", p/s}')
        renderer="n/a (release, no stats)"
        builds="-"
        phases="-"
      fi
    else
      if [ -n "$LINUX_TARGET" ]; then
        build_app "$dir" --debug
        local stats="$dir/stats"
        linux_run "$dir" stress "" "MFB_CANVAS_GPU=1" "MFB_CANVAS_STATS=$stats" || true
        local rendered; rendered=$(wc -l < "$stats" 2>/dev/null | tr -d ' '); rendered=${rendered:-0}
        fps=$(awk -v r="$rendered" -v s="$secs" 'BEGIN{printf "%.1f", r/s}')
        local g0 g1; g0=$(stats_field_line "$(head -1 "$stats" 2>/dev/null)" generations); g1=$(stats_field "$stats" generations)
        local gp1; gp1=$(stats_field "$stats" gpuFrames)
        renderer=$(renderer_of "$rendered" "$gp1")
        builds=$(awk -v a="$g0" -v b="$g1" -v r="$rendered" 'BEGIN{ if (r>0) printf "%.2f", (b-a)/r; else print "-" }')
        phases=$(phase_fields "$stats" "$rendered")
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
  if [ -n "$LINUX_TARGET" ]; then
    build_app "$dir" --debug
    for mode in sw gpu; do
      local gpu=0; [ "$mode" = gpu ] && gpu=1
      linux_run "$dir" poly "" "MFB_CANVAS_SYNC=1" "MFB_CANVAS_GPU=$gpu" \
        "MFB_CANVAS_STATS=$dir/stats-$mode" "MFB_CANVAS_DUMP=$dir/frame-$mode"
    done
  else
    build_app "$dir" --debug
    local bin; bin=$(bin_path "$dir" poly)
    for mode in sw gpu; do
      local gpu=0; [ "$mode" = gpu ] && gpu=1
      env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=$gpu \
        MFB_CANVAS_STATS="$dir/stats-$mode" MFB_CANVAS_DUMP="$dir/frame-$mode" "$bin" >/dev/null 2>&1
    done
  fi
  local cmp; cmp=$(python3 "$CMP" "$dir/frame-sw" "$dir/frame-gpu")
  local status
  if [ -n "$LINUX_TARGET" ]; then
    status=$(tail -1 "$dir/stats-gpu" 2>/dev/null | grep -oE '(gpuSelected|vulkanReady|gpuFrames)=[A-Za-z0-9]+' | tr '\n' ' ')
  else
    status=$(tail -1 "$dir/stats-gpu" 2>/dev/null | grep -oE '(gpuSelected|metalReady|gpuFrames)=[A-Za-z0-9]+' | tr '\n' ' ')
  fi
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
  if [ -n "$LINUX_TARGET" ]; then
    build_app "$live" --debug
    build_app "$oracle" --debug
    linux_run "$live" pics "" "MFB_CANVAS_GPU=1" "MFB_CANVAS_STATS=$live/stats" || true
    for mode in sw gpu; do
      local gpu=0; [ "$mode" = gpu ] && gpu=1
      linux_run "$oracle" pics "" "MFB_CANVAS_SYNC=1" "MFB_CANVAS_GPU=$gpu" \
        "MFB_CANVAS_STATS=$oracle/stats-$mode" "MFB_CANVAS_DUMP=$oracle/frame-$mode"
    done
  else
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
  fi
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
  if [ -n "$LINUX_TARGET" ]; then
    bash "$GEN/text.sh" "$live" "$lines" "$cols" "$size" 5 0 "$LINUX_FONT"
    bash "$GEN/text.sh" "$oracle" "$lines" "$cols" "$size" 0 1 "$LINUX_FONT"
    build_app "$live" --debug
    build_app "$oracle" --debug
    linux_run "$live" txt "" "MFB_CANVAS_GPU=1" "MFB_CANVAS_STATS=$live/stats" || true
    for mode in sw gpu; do
      local gpu=0; [ "$mode" = gpu ] && gpu=1
      linux_run "$oracle" txt "" "MFB_CANVAS_SYNC=1" "MFB_CANVAS_GPU=$gpu" \
        "MFB_CANVAS_STATS=$oracle/stats-$mode" "MFB_CANVAS_DUMP=$oracle/frame-$mode"
    done
  else
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
  fi
  local rendered; rendered=$(wc -l < "$live/stats" | tr -d ' ')
  local live_status; live_status=$(tail -1 "$live/stats" | grep -oE '(gpuFrames|glyphs|glyphEvictions)=[0-9]+' | tr '\n' ' ')
  local phases; phases=$(phase_fields "$live/stats" "$rendered")
  local cmp; cmp=$(python3 "$CMP" "$oracle/frame-sw" "$oracle/frame-gpu")
  local ostatus; ostatus=$(tail -1 "$oracle/stats-gpu" | grep -oE 'gpuFrames=[0-9]+')
  echo "text ${lines}x${cols} chars at ${size}px | rendered=$rendered/5s $live_status phase: $phases | oracle: $cmp $ostatus"
}

# ---------------------------------------------------------------------------
# weather
# ---------------------------------------------------------------------------
# weather quits at the end of its input (`readKey` turns EOF into Quit), and a background
# job's stdin is /dev/null, so launched plainly it draws one frame and exits. Its stdin
# is therefore a FIFO this side holds open for writing: reads block, never EOF.
cmd_weather() {
  local window=${1:-20}
  local dir="$WORK/weather"
  cp -R "$REPO_ROOT/examples/weather" "$dir"
  rm -rf "$dir/build"
  if [ -n "$LINUX_TARGET" ]; then
    build_app "$dir" --debug
    local rdir; rdir=$(remote_dir_of "$dir")
    local rstats="$rdir/stats"
    # `;`, not `&&`, before `setsid`: a `&` after an AND-list backgrounds the whole list,
    # whose shell keeps this ssh session's stdout open until weather exits.
    ssh_box "cd '$rdir' && rm -f stdin.fifo; mkfifo stdin.fifo; setsid sh -c 'exec 3<>stdin.fifo; exec env MFB_GTKAPP_HEADLESS=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS=$rstats ./squashfs-root/usr/bin/weather <&3' >run.log 2>&1 </dev/null & disown; true"
    local waited=0
    local rlines
    rlines=$(ssh_box "wc -l < '$rstats' 2>/dev/null || echo 0" | tr -d ' ')
    until [ "${rlines:-0}" -ge 2 ] 2>/dev/null; do
      sleep 1; waited=$((waited + 1))
      if [ "$waited" -ge 150 ]; then
        ssh_box "pkill -f squashfs-root/usr/bin/weather" >/dev/null 2>&1 || true
        die "weather: no second (first map) frame after 150s"
      fi
      rlines=$(ssh_box "wc -l < '$rstats' 2>/dev/null || echo 0" | tr -d ' ')
    done
    local a; a=$rlines
    local la; la=$(ssh_box "sed -n '${a}p' '$rstats'")
    sleep "$window"
    local b; b=$(ssh_box "wc -l < '$rstats' 2>/dev/null || echo 0" | tr -d ' ')
    local lb; lb=$(ssh_box "sed -n '${b}p' '$rstats'")
    ssh_box "pkill -f squashfs-root/usr/bin/weather" >/dev/null 2>&1 || true
    local frames=$((b - a))
    local fps; fps=$(awk -v f="$frames" -v w="$window" 'BEGIN{printf "%.2f", f/w}')
    echo "waited ${waited}s for first map frame; frames in ${window}s window = $frames (fps=$fps)"
    echo "  start: $(echo "$la" | grep -oE '(gpuFrames|blocks|spike[A-Za-z]+|phase[A-Za-z]+)=[0-9.]+' | tr '\n' ' ')"
    echo "  end:   $(echo "$lb" | grep -oE '(gpuFrames|blocks|generations|spike[A-Za-z]+|phase[A-Za-z]+)=[0-9.]+' | tr '\n' ' ')"
  else
    build_app "$dir" --debug
    local bin; bin=$(bin_path "$dir" weather)
    local stats="$dir/stats"
    mkfifo "$dir/stdin.fifo"
    exec 3<>"$dir/stdin.fifo"
    ( env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS="$stats" "$bin" <&3 >/dev/null 2>&1 ) &
    local pid=$!
    local waited=0
    until [ -f "$stats" ] && [ "$(wc -l < "$stats" | tr -d ' ')" -ge 2 ]; do
      sleep 1; waited=$((waited + 1))
      if [ "$waited" -ge 150 ]; then
        kill "$pid" 2>/dev/null || true
        die "weather: no second (first map) frame after 150s"
      fi
    done
    local a; a=$(wc -l < "$stats" | tr -d ' ')
    local la; la=$(sed -n "${a}p" "$stats")
    sleep "$window"
    local b; b=$(wc -l < "$stats" | tr -d ' ')
    local lb; lb=$(sed -n "${b}p" "$stats")
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    exec 3>&-
    local frames=$((b - a))
    local fps; fps=$(awk -v f="$frames" -v w="$window" 'BEGIN{printf "%.2f", f/w}')
    echo "waited ${waited}s for first map frame; frames in ${window}s window = $frames (fps=$fps)"
    echo "  start: $(echo "$la" | grep -oE '(gpuFrames|blocks|spike[A-Za-z]+|phase[A-Za-z]+)=[0-9.]+' | tr '\n' ' ')"
    echo "  end:   $(echo "$lb" | grep -oE '(gpuFrames|blocks|generations|spike[A-Za-z]+|phase[A-Za-z]+)=[0-9.]+' | tr '\n' ' ')"
  fi
}

# ---------------------------------------------------------------------------
# worker
# ---------------------------------------------------------------------------
cmd_worker() {
  local dir="$WORK/worker"
  bash "$GEN/worker.sh" "$dir"
  if [ -n "$LINUX_TARGET" ]; then
    build_app "$dir"
    local outfile="$dir/run.out"
    linux_run "$dir" mb "$outfile" "MFB_CANVAS_GPU=0"
    cat "$outfile"
  else
    build_app "$dir"
    local bin; bin=$(bin_path "$dir" mb)
    env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=0 "$bin"
  fi
}

# ---------------------------------------------------------------------------
# hit
# ---------------------------------------------------------------------------
# One row per (items, ids): the median and the minimum, in nanoseconds over the
# 40 frames `gen/hit.sh` runs, of a present, of the first poll after it and of
# each further poll against the same scene; then the `overID` the last poll
# reported and how many of the last frame's sixty presses arrived.
hit_stat() {
  # hit_stat FILE LABEL -- "median/min" of the numbers on FILE's LABEL line.
  local nums
  nums=$(grep "^$2" "$1" | cut -d: -f2 | tr ' ' '\n' | grep -E '^[0-9]+$' | sort -n)
  [ -n "$nums" ] || { echo "-/-"; return; }
  local count; count=$(echo "$nums" | wc -l | tr -d ' ')
  echo "$(echo "$nums" | sed -n "$(( (count + 1) / 2 ))p")/$(echo "$nums" | head -1)"
}

cmd_hit() {
  [ "$WIN" != 1 ] || die "hit: MFB_MOUSE_INJECT is POSIX-only, so there is no event to poll on Windows"
  # Sixty presses of button 0 at pixel (444, 54): SGR-1006, 1-based column;row.
  # The ring holds 64.
  local n ids
  printf '%-7s %-4s %-18s %-20s %-20s %-9s %s\n' items ids "present(med/min)" "poll-first(med/min)" "poll-again(med/min)" overID events
  for n in 1000 10000; do
    for ids in 1 0; do
      local dir="$WORK/hit-$n-$ids" outfile
      outfile="$dir/run.out"
      bash "$GEN/hit.sh" "$dir" "$n" "$ids"
      build_app "$dir"
      if [ -n "$LINUX_TARGET" ]; then
        # The value is built on the box: an escape and a `;` do not survive the
        # unquoted `env K=V` line `linux_run` sends.
        linux_run "$dir" hit "$outfile" "MFB_CANVAS_GPU=0" "MFB_MOUSE_INJECT=\$(for i in \$(seq 60); do printf '\\033[<0;445;55M'; done)"
      else
        local bin; bin=$(bin_path "$dir" hit)
        env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=0 MFB_MOUSE_INJECT="$(for i in $(seq 60); do printf '\033[<0;445;55M'; done)" "$bin" > "$outfile" 2>&1
      fi
      printf '%-7s %-4s %-18s %-20s %-20s %-9s %s\n' "$n" "$ids" \
        "$(hit_stat "$outfile" present-ns)" "$(hit_stat "$outfile" poll-first-ns)" \
        "$(hit_stat "$outfile" poll-again-ns)" "$(grep '^over:' "$outfile" | cut -d' ' -f2)" \
        "$(grep '^events:' "$outfile" | cut -d' ' -f2)"
    done
  done
}

# ---------------------------------------------------------------------------
# compare
# ---------------------------------------------------------------------------
# clips (bug-995)
# ---------------------------------------------------------------------------
# clips_ms STATS -- ms a GPU frame after the first: the render phase timer is
# cumulative, so (last - first) / (frames - 1) leaves out the first frame and
# whatever pipeline it compiled.
clips_ms() {
  local stats=$1
  local n; n=$(wc -l < "$stats" 2>/dev/null | tr -d ' '); n=${n:-0}
  if [ "$n" -lt 2 ]; then echo "-"; return; fi
  local a b; a=$(stats_field_line "$(head -1 "$stats")" phaseRenderMs); b=$(stats_field "$stats" phaseRenderMs)
  awk -v a="$a" -v b="$b" -v n="$n" 'BEGIN{printf "%.1f", (b-a)/(n-1)}'
}

# One row: build the probe, run it on the GPU under MFB_CANVAS_SYNC, print ms/frame.
clips_row() {
  local items=$1 points=$2 scale=$3 frames=$4 mode=$5
  local dir="$WORK/clips-$points-$scale-$mode"
  bash "$GEN/clips.sh" "$dir" "$items" "$points" "$scale" "$frames" "$mode"
  build_app "$dir" --debug
  local stats="$dir/stats"
  if [ -n "$LINUX_TARGET" ]; then
    # On Windows the first run of a new program compiles its pipelines (bug-996):
    # run it once to warm the driver's cache and read the second run.
    if [ "$WIN" = 1 ]; then linux_run "$dir" clips "" "MFB_CANVAS_GPU=1" "MFB_CANVAS_SYNC=1" || true; fi
    linux_run "$dir" clips "" "MFB_CANVAS_GPU=1" "MFB_CANVAS_SYNC=1" "MFB_CANVAS_STATS=$stats" || true
  else
    env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_STATS="$stats" \
      "$(bin_path "$dir" clips)" >/dev/null 2>&1 || true
  fi
  local rendered; rendered=$(wc -l < "$stats" 2>/dev/null | tr -d ' '); rendered=${rendered:-0}
  local gpu; gpu=$(stats_field "$stats" gpuFrames); gpu=${gpu:-0}
  local label="untransformed"
  [ "$scale" != 0 ] && label="$mode scale $scale"
  printf "clip points=%-3s %-18s | ms/frame=%-7s renderer=%-8s frames=%s\n" \
    "$points" "$label" "$(clips_ms "$stats")" "$(renderer_of "$rendered" "$gpu")" "$rendered"
}

cmd_clips() {
  local items=${1:-150}; local frames=${2:-40}
  echo "# $items squares of 100 px, --debug build, MFB_CANVAS_SYNC=1; ms/frame is the render phase after the first frame"
  local row
  for row in "0 0 item" "64 0 item" "28 0 item" \
             "0 4 item" "64 4 item" "28 4 item" \
             "0 0.1 item" "64 0.1 item" "28 0.1 item" \
             "0 4 group" "64 4 group" "28 4 group"; do
    # shellcheck disable=SC2086
    set -- $row
    clips_row "$items" "$1" "$2" "$frames" "$3"
  done
}

# ---------------------------------------------------------------------------
compare_run() {
  # compare_run DIR BINNAME -- sw vs gpu dump on an already-generated project.
  local dir=$1 name=$2
  if [ -n "$LINUX_TARGET" ]; then
    build_app "$dir" --debug
    for mode in sw gpu; do
      local gpu=0; [ "$mode" = gpu ] && gpu=1
      linux_run "$dir" "$name" "" "MFB_CANVAS_SYNC=1" "MFB_CANVAS_GPU=$gpu" \
        "MFB_CANVAS_STATS=$dir/stats-$mode" "MFB_CANVAS_DUMP=$dir/frame-$mode"
    done
  else
    build_app "$dir" --debug
    local bin; bin=$(bin_path "$dir" "$name")
    for mode in sw gpu; do
      local gpu=0; [ "$mode" = gpu ] && gpu=1
      env MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=$gpu \
        MFB_CANVAS_STATS="$dir/stats-$mode" MFB_CANVAS_DUMP="$dir/frame-$mode" "$bin" >/dev/null 2>&1
    done
  fi
  local cmp; cmp=$(python3 "$CMP" "$dir/frame-sw" "$dir/frame-gpu")
  local status
  if [ -n "$LINUX_TARGET" ]; then
    status=$(tail -1 "$dir/stats-gpu" 2>/dev/null | grep -oE '(gpuSelected|vulkanReady|gpuFrames)=[A-Za-z0-9]+' | tr '\n' ' ')
  else
    status=$(tail -1 "$dir/stats-gpu" 2>/dev/null | grep -oE '(gpuSelected|metalReady|gpuFrames)=[A-Za-z0-9]+' | tr '\n' ' ')
  fi
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
      if [ -n "$LINUX_TARGET" ]; then
        bash "$GEN/text.sh" "$dir" "$lines" "$cols" "$size" 0 1 "$LINUX_FONT"
      else
        bash "$GEN/text.sh" "$dir" "$lines" "$cols" "$size" 0 1
      fi
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
  weather) cmd_weather "$@" ;;
  worker) cmd_worker "$@" ;;
  hit) cmd_hit "$@" ;;
  clips) cmd_clips "$@" ;;
  compare) cmd_compare "$@" ;;
  *) usage ;;
esac

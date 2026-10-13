#!/usr/bin/env bash
# The pointer words on the targets the Mac does not run (plan-172-J).
#
# Each platform's main thread publishes where the pointer is into the canvas graphics
# state, and the graphics thread reads it (`mfb spec app canvas`, "The pointer, as the
# graphics thread sees it"). `tests/canvas/rt_canvas_pointer.rs` holds that on the host.
# This runs the same question on a box, for the two things a host run cannot reach:
#
#   * the writer and the reader as THIS target's emitter lowers them — the sequence
#     word's release stores on AArch64, plain stores on x86-64 — driven by the
#     `--debug` build's script, `MFB_CANVAS_POINTER`;
#   * the platform's own pointer callback, called headless with a scripted position
#     through `MFB_CANVAS_POINTER_X` / `_Y`, on the scripted-resize precedent: it runs
#     the callback the window system would call, with the same argument shape.
#
# One program, cross-built here as a `--debug` app and shipped: three presents of a
# changing scene, 2.5 s apart, so there is a frame before the scripted move and frames
# after it. Each run's `pointer=` fields are read back from `MFB_CANVAS_STATS`.
#
# What it does NOT prove: that a real window delivers the event to the callback. That
# needs a display and a hand on the pointer (`scripts/test-macapp.sh` case 6c is that
# proof for macOS).
#
# Usage: scripts/test-canvas-pointer.sh <mfb-exe> [--target <t>] [--box <port>]
#                                       [--libc glibc|musl]
#
#   --target  linux-aarch64 (default, box 2226), linux-x86_64 (box 2227, musl) or
#             windows-x86_64 (box 206, `test@192.168.1.206`).
#   --box     the ssh port, when it is not the target's default box.
#   --libc    which AppImage to ship on Linux; defaults to the default box's.
set -u
. "$(dirname "$0")/remote-common.sh"

if [ "$#" -lt 1 ]; then
  echo "usage: test-canvas-pointer.sh <mfb-exe> [--target <t>] [--box <port>] [--libc glibc|musl]" >&2
  exit 2
fi
MFB_EXE=$1
shift
TARGET=linux-aarch64
PORT=""
LIBC=""
while [ $# -gt 0 ]; do
  case "$1" in
    --target) TARGET=$2; shift 2 ;;
    --box) PORT=$2; shift 2 ;;
    --libc) LIBC=$2; shift 2 ;;
    *) echo "test-canvas-pointer.sh: unknown argument $1" >&2; exit 2 ;;
  esac
done
host=test@127.0.0.1
case "$TARGET" in
  linux-aarch64) : "${PORT:=2226}"; : "${LIBC:=glibc}" ;;
  linux-x86_64) : "${PORT:=2227}"; : "${LIBC:=musl}" ;;
  windows-x86_64) : "${PORT:=22}"; host=test@192.168.1.206 ;;
  *) echo "test-canvas-pointer.sh: unsupported target $TARGET" >&2; exit 2 ;;
esac
windows=0
[ "$TARGET" = windows-x86_64 ] && windows=1

rc_workdir
proj="$work/pointer"
mkdir -p "$proj/src"
scaffold_project "$proj" pointer
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
IMPORT canvas
IMPORT color
IMPORT os
IMPORT vector
FUNC box(x AS Float) AS canvas::DrawItem
  MUT r AS canvas::Rectangle
  r = WITH r { position := vector::Float2[x := x, y := 100.0], size := canvas::Size[width := 50.0, height := 50.0], paint := canvas::fill(color::rgb(200, 200, 200)) }
  LET item AS canvas::DrawItem = r
  RETURN item
END FUNC
FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  canvas::present([box(100.0)])
  os::sleep(2500)
  canvas::present([box(200.0)])
  os::sleep(2500)
  canvas::present([box(300.0)])
END FUNC
MFB
if ! "$MFB_EXE" build --app --debug --target "$TARGET" "$proj" >"$work/build.log" 2>&1; then
  fail "build --app --debug --target $TARGET"
  cat "$work/build.log" >&2
  exit 1
fi

if [ "$windows" = 1 ]; then
  rdir="C:\\mfbpointer-$$"
  win_ship "$PORT" "$host" "$rdir" "$proj/build/pointer.exe"
else
  rdir="/tmp/mfb-pointer-$$"
  remote_ssh "$PORT" "$host" "rm -rf $rdir && mkdir -p $rdir"
  remote_scp "$PORT" "$proj/build/pointer-$LIBC.AppImage" "$host:$rdir/app.AppImage" >/dev/null
  remote_ssh "$PORT" "$host" "cd $rdir && ./app.AppImage --appimage-extract >/dev/null 2>&1"
fi

# usage: run <tag> <ENV=value>... -- sets `seen` to the run's distinct pointer= values.
run() {
  local tag=$1
  shift
  if [ "$windows" = 1 ]; then
    {
      printf '@echo off\r\n'
      printf 'set MFB_HEADLESS=1\r\nset MFB_CANVAS_SYNC=1\r\n'
      printf 'set MFB_CANVAS_STATS=%s\\%s.txt\r\n' "$rdir" "$tag"
      local e
      for e in "$@"; do printf 'set %s\r\n' "$e"; done
      printf '%s\\pointer.exe\r\n' "$rdir"
    } > "$work/$tag.bat"
    remote_scp "$PORT" "$work/$tag.bat" "$host:C:/mfbpointer-$$/$tag.bat" >/dev/null
    remote_ssh "$PORT" "$host" "$rdir\\$tag.bat" >/dev/null 2>&1 || true
    remote_scp "$PORT" "$host:C:/mfbpointer-$$/$tag.txt" "$work/$tag.txt" >/dev/null 2>&1 || true
  else
    remote_ssh "$PORT" "$host" "cd $rdir && timeout 60 env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_STATS=$rdir/$tag.txt $* ./squashfs-root/usr/bin/pointer >/dev/null 2>&1" || true
    remote_scp "$PORT" "$host:$rdir/$tag.txt" "$work/$tag.txt" >/dev/null 2>&1 || true
  fi
  seen=$(tr -d '\r' < "$work/$tag.txt" 2>/dev/null | grep -o 'pointer=[0-9,-]*' | uniq | tr '\n' ' ')
}

# 1. The script: the writer and the reader as this target lowers them.
run script "MFB_CANVAS_POINTER=0,10,20,5,1"
if [ "$seen" = "pointer=10,20,5,1 " ]; then
  pass "$TARGET: the graphics thread read the scripted pointer (10,20 buttons 5 inside)"
else
  fail "$TARGET: scripted pointer: saw '$seen'"
fi

# 2. The platform's callback, inside the surface: nothing before the first frame, the
#    position after it.
# A headless run has no window to publish a surface size, and "inside" is asked of the
#    published size -- so the size is scripted too, as a window would have published it.
run inside "MFB_CANVAS_RESIZE_W=900" "MFB_CANVAS_RESIZE_H=640" "MFB_CANVAS_POINTER_X=123" "MFB_CANVAS_POINTER_Y=45"
if [ "$seen" = "pointer=0,0,0,0 pointer=123,45,0,1 " ]; then
  pass "$TARGET: the platform callback published (123,45) as inside"
else
  fail "$TARGET: platform callback, inside: saw '$seen'"
fi

# 3. The same callback with a point off the 900x640 surface: published, as outside.
run outside "MFB_CANVAS_RESIZE_W=900" "MFB_CANVAS_RESIZE_H=640" "MFB_CANVAS_POINTER_X=5000" "MFB_CANVAS_POINTER_Y=45"
if [ "$seen" = "pointer=0,0,0,0 pointer=5000,45,0,0 " ]; then
  pass "$TARGET: the platform callback published (5000,45) as outside"
else
  fail "$TARGET: platform callback, outside: saw '$seen'"
fi

if [ "$windows" = 1 ]; then
  remote_ssh "$PORT" "$host" "rmdir /s /q $rdir" >/dev/null 2>&1 || true
else
  remote_ssh "$PORT" "$host" "rm -rf $rdir"
fi
if [ "$rc_failures" -ne 0 ]; then
  echo "canvas pointer rows failed on $TARGET: $rc_failures" >&2
  exit 1
fi
echo "canvas pointer rows passed on $TARGET"

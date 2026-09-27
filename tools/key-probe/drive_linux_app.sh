#!/bin/sh
# Drive the --app key probe on a Linux GTK box (tools/key-probe).
#
#   sh drive_linux_app.sh <keyprobe-*.AppImage> <out-file> [transcript|tui|canvas]
#
# Runs ON the box. Extracts the AppImage (no FUSE needed), starts a private
# Xvfb, launches the app under X11 with the Cairo renderer, focuses its window
# and types the probe's key sequence with xdotool (XTEST: real key events
# through the X server and GTK). Prints the probe's output file. Needs Xvfb and
# xdotool.
set -eu
appimage=$1
out=$2
surface=${3:-transcript}
work=$(mktemp -d)
trap 'kill $app $xvfb 2>/dev/null || true; rm -rf "$work"' EXIT
cp "$appimage" "$work/app.AppImage"
chmod +x "$work/app.AppImage"
(cd "$work" && ./app.AppImage --appimage-extract >/dev/null)
display=:$(( ($$ % 200) + 50 ))
Xvfb "$display" -screen 0 1280x800x24 >/dev/null 2>&1 &
xvfb=$!
sleep 1
rm -f "$out"
DISPLAY=$display GDK_BACKEND=x11 GSK_RENDERER=cairo \
  KEYPROBE_OUT=$out KEYPROBE_SURFACE=$surface \
  "$work/squashfs-root/AppRun" >"$work/app.log" 2>&1 &
app=$!
i=0
while [ ! -f "$out" ] && [ $i -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
win=$(DISPLAY=$display xdotool search --sync --onlyvisible --pid "$app" 2>/dev/null | head -1 || true)
if [ -z "$win" ]; then
  win=$(DISPLAY=$display xdotool search --sync --onlyvisible --name '.' | head -1)
fi
sleep 1
DISPLAY=$display xdotool windowfocus --sync "$win"
# Plain `xdotool key` types through XTEST into the focused window, so the
# events carry the X server's real modifier state (`--window` would send
# synthetic events instead).
# Shift+Left is spelled out as downs and ups: `xdotool key shift+Left`
# releases the keys in the order it pressed them, so Left would come up after
# Shift and its release carry no Shift — not what a person's fingers do.
for key in a Left Right Up Down BackSpace Delete Escape F5 shift+Left Home End eacute q; do
  if [ "$key" = shift+Left ]; then
    DISPLAY=$display xdotool keydown shift keydown Left
    sleep 0.05
    DISPLAY=$display xdotool keyup Left keyup shift
  else
    DISPLAY=$display xdotool key --delay 50 "$key"
  fi
  sleep 0.15
done
i=0
while kill -0 "$app" 2>/dev/null && [ $i -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
cat "$out"

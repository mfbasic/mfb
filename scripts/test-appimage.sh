#!/usr/bin/env bash
# Runtime acceptance for Linux app mode (plan-51-D §4.2).
#
# The Linux counterpart of `scripts/test-macapp.sh`, with one structural
# difference: the artifact has to travel. The dev host is macOS and cannot run a
# Linux binary — and, critically, **cannot emulate one either**. An AppImage
# carries hex 0x414902 at offset 8 (EI_ABIVERSION + EI_PAD); the real Linux
# kernel ignores those bytes, but qemu-user's and Rosetta's ELF loaders reject
# them outright, so an AppImage under Docker/binfmt fails with `applet not found`
# before its runtime ever runs (plan-51-C §4.6). Proof needs a real kernel.
#
# So: build here, ship over ssh, run there, assert, clean up.
#
# Proves: the AppImage runtime mounts our SquashFS via its bundled squashfuse,
# `execv`s AppRun, the GTK4 program boots and runs the MFBASIC entry, vendored
# libraries resolve from inside the sealed image, and the extracted payload
# matches the AppDir the seal consumed.
#
# Usage: scripts/test-appimage.sh <mfb-exe> [--box <port>] [--gui]
#
#   --box <port>  ssh port of the target box (default 2228, Ubuntu x86_64 GTK).
#                 The GTK boxes are the only ones that matter: app mode is
#                 glibc-only, so the Alpine/musl boxes are out of scope by
#                 construction, and riscv64 has no upstream runtime at all.
#   --gui         additionally run the windowed case, which needs a real display
#                 on the box. Opt-in, like MFB_MACAPP_GUI=1.
set -u

MFB_EXE=${1:-}
if [ -z "$MFB_EXE" ]; then
  echo "usage: test-appimage.sh <mfb-exe> [--box <port>] [--gui]" >&2
  exit 2
fi
shift

BOX_PORT=2228
GUI=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --box) BOX_PORT=$2; shift 2 ;;
    --gui) GUI=1; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

# Each box's arch, so the build targets what the box can actually execute.
case "$BOX_PORT" in
  2228) TARGET=linux-x86_64 ;;   # Ubuntu x86_64 GTK
  2226) TARGET=linux-aarch64 ;;  # Debian 12 GTK
  *) echo "unknown box port $BOX_PORT; expected 2226 or 2228" >&2; exit 2 ;;
esac

ROOT=$(cd "$(dirname "$0")/.." && pwd)
# The vendor case reuses the libsnd binding's real blobs rather than inventing
# one, so the bytes travelling into the image are bytes a real project ships.
case "$TARGET" in
  linux-x86_64)  VENDOR_BLOB=libsndfile.so.1.0.37-x86_64-glibc ;;
  linux-aarch64) VENDOR_BLOB=libsndfile.so.1.0.37-aarch64-glibc ;;
esac
VENDOR_SRC="$ROOT/bindings/libsnd/vendor/$VENDOR_BLOB"

SSH="ssh -o ConnectTimeout=8 -o BatchMode=yes -p $BOX_PORT test@127.0.0.1"
SCP="scp -q -o ConnectTimeout=8 -o BatchMode=yes -P $BOX_PORT"

if ! $SSH true 2>/dev/null; then
  # The GTK boxes are developer infrastructure, not CI. A suite that goes red for
  # infrastructure reasons trains people to ignore red bars, so an unreachable
  # box is a skip — but a *reachable* box that fails is a failure.
  echo "skip: box on port $BOX_PORT is not reachable" >&2
  exit 0
fi

work=$(mktemp -d)
remote=/tmp/mfb-appimage-$$
trap 'rm -rf "$work"; $SSH "rm -rf $remote" >/dev/null 2>&1 || true' EXIT
$SSH "mkdir -p $remote" || exit 1
failures=0

pass() { echo "ok: $1"; }
fail() { echo "FAIL: $1" >&2; failures=$((failures + 1)); }

# Run a command on the box under a 15s watchdog. A GUI app that fails to start
# does not exit — it hangs — and a hung ssh in an acceptance script is a wedged
# terminal. Prints the command's output; exits 99 on timeout.
#
# `-n` keeps ssh from consuming this script's stdin.
box_run() {
  timeout_run 15 $SSH -n "$1"
}

timeout_run() {
  local limit=$1; shift
  perl -e '
    my $limit = shift @ARGV;
    my $pid = open(my $fh, "-|");
    if (!defined $pid) { exit 98; }
    if ($pid == 0) { exec(@ARGV) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; waitpid($pid, 0); exit 99; };
    alarm $limit;
    local $/; my $out = <$fh>; close($fh); my $st = $?;
    print $out if defined $out;
    exit($st >> 8);
  ' "$limit" "$@"
}

# Build a one-file project and return its AppImage path, or empty on failure.
# `$1` = project name, `$2` = MFBASIC source, `$3...` = extra mfb build flags.
build_appimage() {
  local name=$1 source=$2; shift 2
  local proj="$work/$name"
  mkdir -p "$proj/src"
  cat > "$proj/project.json" <<JSON
{ "name": "$name", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
  printf '%s' "$source" > "$proj/src/main.mfb"
  if ! "$MFB_EXE" build -q --app -target "$TARGET" "$@" "$proj" >/dev/null 2>&1; then
    return 1
  fi
  printf '%s' "$proj/build/$name.AppImage"
}

# --- Case 1: it mounts and the inner program actually starts -----------------
#
# ⚠️ "Did not crash" is NOT the assertion. Two facts shape what can be checked
# without a display:
#
#   * `_mfb_gtkapp_main` calls `g_application_run`, which fails outright when no
#     display is reachable — it does not fall through to the GTK backend's fd
#     fallback, so the MFBASIC entry never runs and the program's own stdout
#     marker never appears. (Verified on box 2228, whose X displays belong to
#     sddm and are not reachable over ssh; `xvfb-run` is not installed.)
#   * In *windowed* mode the finish contract parks the worker in `pause()` so the
#     window stays open, meaning a successful program does not produce a
#     successful process exit. A script asserting a clean exit would pass exactly
#     when the app crashed before opening a window.
#
# So the display-less assertion is that the program reached GTK's display probe:
# that requires the FUSE mount to have succeeded, `AppRun` to have been followed,
# and the inner ELF to have dynamically linked libgtk-4 and started running. When
# a display IS available the stronger marker assertion applies, and Case 6 covers
# the windowed path. Either signal counts as a pass; neither one appearing is a
# failure.
started_ok() {
  printf '%s' "$1" | grep -qE "appimage-marker-ok|Failed to open display|Gtk-WARNING"
}
HELLO_SRC='IMPORT io

SUB main()
  io::print("appimage-marker-ok")
END SUB
'
image=$(build_appimage hello "$HELLO_SRC") || image=""
if [ -z "$image" ]; then
  fail "build --app -target $TARGET (hello)"
else
  $SCP "$image" "test@127.0.0.1:$remote/hello.AppImage" || fail "scp hello.AppImage"
  # The build already set 0755; confirm that survived rather than re-chmodding,
  # since an AppImage without the executable bit gets no diagnostic beyond
  # "Permission denied".
  mode=$(box_run "stat -c %a $remote/hello.AppImage" | tr -d '\r\n')
  if [ "$mode" = "755" ]; then
    pass "the shipped AppImage is mode 0755"
  else
    fail "expected mode 755, got '$mode'"
  fi

  out=$(box_run "cd $remote && ./hello.AppImage 2>&1")
  status=$?
  if [ "$status" -eq 99 ]; then
    fail "hello.AppImage hung (watchdog fired) — the FUSE mount or the GTK boot wedged"
  elif started_ok "$out"; then
    pass "the AppImage mounted via FUSE and started the inner GTK program"
  else
    fail "hello.AppImage never reached the GTK bootstrap (exit $status): $out"
  fi

  # --- Case 2: the no-FUSE path ---------------------------------------------
  #
  # The runtime has NO automatic fallback: if the mount fails it prints "Cannot
  # mount AppImage, please check your FUSE setup" and exits. Self-extraction is
  # opt-in, and it exercises the same squashfs through the same reader.
  out=$(box_run "cd $remote && ./hello.AppImage --appimage-extract-and-run 2>&1")
  if started_ok "$out"; then
    pass "--appimage-extract-and-run works (the no-FUSE path)"
  else
    fail "--appimage-extract-and-run never reached the GTK bootstrap: $out"
  fi

  # --- Case 3: the extracted tree matches the AppDir the seal consumed -------
  dbg=$(build_appimage hellodbg "$HELLO_SRC" --app-debug) || dbg=""
  if [ -z "$dbg" ]; then
    fail "build --app-debug"
  elif [ ! -d "$work/hellodbg/build/hellodbg.AppDir" ]; then
    fail "--app-debug did not keep the AppDir"
  else
    $SCP "$dbg" "test@127.0.0.1:$remote/hellodbg.AppImage" || fail "scp hellodbg.AppImage"
    box_run "cd $remote && rm -rf squashfs-root && ./hellodbg.AppImage --appimage-extract >/dev/null 2>&1" >/dev/null
    # Compare the payload by content hash of every regular file plus every
    # symlink's target — a plain `diff -r` would follow AppRun and report the
    # ELF twice.
    # LC_ALL=C: macOS and Linux `sort` disagree on whether `.DirIcon` precedes
    # `AppRun`, and a locale-dependent order would make this compare fail for a
    # reason that has nothing to do with the payload.
    local_manifest=$(cd "$work/hellodbg/build/hellodbg.AppDir" && \
      find . \( -type f -o -type l \) | LC_ALL=C sort | while read -r p; do
        if [ -L "$p" ]; then printf '%s -> %s\n' "$p" "$(readlink "$p")";
        else printf '%s %s\n' "$p" "$(shasum -a 256 "$p" | cut -d' ' -f1)"; fi
      done)
    remote_manifest=$($SSH -n "cd $remote/squashfs-root && \
      find . \\( -type f -o -type l \\) | LC_ALL=C sort | while read -r p; do \
        if [ -L \"\$p\" ]; then printf '%s -> %s\\n' \"\$p\" \"\$(readlink \"\$p\")\"; \
        else printf '%s %s\\n' \"\$p\" \"\$(sha256sum \"\$p\" | cut -d' ' -f1)\"; fi; \
      done")
    if [ "$local_manifest" = "$remote_manifest" ]; then
      pass "--appimage-extract reproduces the --app-debug AppDir exactly"
    else
      fail "extracted tree differs from the AppDir:
$(diff <(printf '%s\n' "$local_manifest") <(printf '%s\n' "$remote_manifest") | head -20)"
    fi

    # --- Case 4: the .desktop entry is valid ---------------------------------
    if $SSH -n "command -v desktop-file-validate" >/dev/null 2>&1; then
      out=$($SSH -n "desktop-file-validate $remote/squashfs-root/hellodbg.desktop 2>&1")
      if [ -z "$out" ]; then
        pass "desktop-file-validate accepts the generated .desktop"
      else
        fail "desktop-file-validate rejected the .desktop: $out"
      fi
    else
      echo "skip: desktop-file-validate not on the box"
    fi
  fi

  # --- Case 5: a corrupted AppImage must FAIL, not pass and not hang ---------
  #
  # This is the script's own acceptance criterion (plan-51-D §3): a green light
  # that cannot go red is worse than none.
  #
  # ⚠️ Truncating by ONE byte is not a corruption. The squashfs is padded to a
  # 4096-byte sector and `bytes_used` deliberately excludes the padding, so
  # lopping off trailing zeros changes nothing a reader looks at — verified on
  # box 2228, where a one-byte-short AppImage mounted and ran perfectly. That is
  # the format working as designed; it just makes for a useless negative test.
  # Corrupt the squashfs magic instead, which is unambiguously fatal.
  runtime_len=$(box_run "cd $remote && python3 -c \"
import struct,sys
b=open('hello.AppImage','rb').read(64)
shoff=struct.unpack_from('<Q',b,0x28)[0]
shentsize=struct.unpack_from('<H',b,0x3a)[0]
shnum=struct.unpack_from('<H',b,0x3c)[0]
print(shoff+shentsize*shnum)\"" | tr -d '\r\n')
  if [ -z "$runtime_len" ]; then
    fail "could not compute the runtime length for the corruption case"
  else
    box_run "cd $remote && cp hello.AppImage broken.AppImage && \
             printf '\\x00' | dd of=broken.AppImage bs=1 seek=$runtime_len count=1 \
               conv=notrunc 2>/dev/null" >/dev/null
    out=$(box_run "cd $remote && ./broken.AppImage 2>&1")
    status=$?
    if [ "$status" -eq 99 ]; then
      fail "a corrupted AppImage hung instead of failing"
    elif started_ok "$out"; then
      fail "a corrupted AppImage still mounted — this script cannot go red"
    else
      pass "a corrupted squashfs superblock fails the mount (the script can go red)"
    fi
  fi
fi

# --- Case 6: a vendored library resolves from inside the sealed image --------
#
# plan-51-A §4.4: an app build's executable sits at `usr/bin/<name>`, one
# directory below its libraries, so it carries `$ORIGIN/../lib` rather than the
# console build's `$ORIGIN/vendor`. The library must be inside the image before
# the seal closes it, and must resolve with **no `LD_LIBRARY_PATH`**.
if [ ! -f "$VENDOR_SRC" ]; then
  echo "skip: vendor case ($VENDOR_SRC not present)"
else
  proj="$work/vend"
  mkdir -p "$proj/src" "$proj/vendor"
  cp "$VENDOR_SRC" "$proj/vendor/$VENDOR_BLOB"
  blob_hash=$(shasum -a 256 "$VENDOR_SRC" | cut -d' ' -f1)
  arch=${TARGET#linux-}
  cat > "$proj/project.json" <<JSON
{ "name": "vend", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "libraries": { "snd": [ { "os": "linux", "arch": "$arch", "libc": "glibc",
    "type": "vendor", "source": "$VENDOR_BLOB", "hash": "$blob_hash" } ] },
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
  cat > "$proj/src/main.mfb" <<'MFB'
IMPORT io

LINK "snd" AS snd
  FUNC probe(handle AS Integer, command AS Integer, data AS Integer, datasize AS Integer) AS Integer
    SYMBOL "sf_command"
    ABI (handle CInt64, command CInt32, data CInt64, datasize CInt32) AS status CInt32
    RETURN status
  END FUNC
END LINK

SUB main()
  io::print("vendored-marker-ok " & toString(snd::probe(0, 0, 0, 0)))
END SUB
MFB
  if ! "$MFB_EXE" build -q --app -target "$TARGET" "$proj" >/dev/null 2>&1; then
    fail "build --app with a vendored library"
  else
    $SCP "$proj/build/vend.AppImage" "test@127.0.0.1:$remote/vend.AppImage" \
      || fail "scp vend.AppImage"
    box_run "cd $remote && rm -rf vroot && mkdir vroot && cd vroot && \
             ../vend.AppImage --appimage-extract >/dev/null 2>&1" >/dev/null

    # The library travelled inside the sealed image, not beside it.
    if $SSH -n "test -f $remote/vroot/squashfs-root/usr/lib/vend-$VENDOR_BLOB"; then
      pass "the vendored library is inside the sealed image at usr/lib/"
    else
      fail "the vendored library is missing from usr/lib/ inside the image"
    fi

    runpath=$($SSH -n "readelf -d $remote/vroot/squashfs-root/usr/bin/vend 2>/dev/null \
                       | sed -n 's/.*RUNPATH.*\[\(.*\)\].*/\1/p'" | tr -d '\r\n')
    if [ "$runpath" = '$ORIGIN/../lib' ]; then
      pass "the app-mode executable carries DT_RUNPATH \$ORIGIN/../lib"
    else
      fail "expected DT_RUNPATH \$ORIGIN/../lib, got '$runpath'"
    fi

    # And the loader actually expands it to the directory the library is in.
    # `$ORIGIN` is expanded by the loader, never by the build, so this is the
    # only place that relationship can be observed.
    trace=$($SSH -n "cd $remote/vroot/squashfs-root && unset LD_LIBRARY_PATH && \
                     LD_DEBUG=libs timeout 15 ./usr/bin/vend 2>&1 \
                     | grep -m1 'RUNPATH from file'")
    if printf '%s' "$trace" | grep -q "usr/bin/../lib"; then
      pass "the loader expands \$ORIGIN/../lib to the image's usr/lib (no LD_LIBRARY_PATH)"
    else
      fail "the loader did not search usr/lib via RUNPATH: $trace"
    fi
  fi
fi

# --- Case 7 (GUI, opt-in): a real window ------------------------------------
#
# Needs a display on the box. In windowed mode the watchdog firing is the
# EXPECTED outcome — the finish contract parks the worker in `pause()` so the
# window stays open — which is the inverse of every other case here.
if [ "$GUI" -eq 1 ]; then
  if [ -z "$image" ]; then
    echo "skip: GUI case (no AppImage built)"
  else
    # Find a display this ssh session can actually open. On box 2228 the X
    # servers belong to sddm and reject an unauthorized client, so the presence
    # of /tmp/.X11-unix/X<n> proves nothing on its own.
    display=$($SSH -n 'for d in ${MFB_APPIMAGE_DISPLAY:-} :0 :1 :2 :3 :4; do
                         [ -n "$d" ] || continue
                         if timeout 3 env DISPLAY=$d xdpyinfo >/dev/null 2>&1; then
                           echo "$d"; break
                         fi
                       done' 2>/dev/null | tr -d '\r\n')
    if [ -z "$display" ]; then
      echo "skip: GUI case (no reachable X display on the box; set MFB_APPIMAGE_DISPLAY)"
    else
      out=$(box_run "cd $remote && DISPLAY=$display ./hello.AppImage 2>&1")
      status=$?
      # In windowed mode the watchdog firing is the EXPECTED outcome: the finish
      # contract parks the worker in `pause()` so the window stays open. This is
      # the inverse of every other case here.
      if [ "$status" -eq 99 ]; then
        pass "windowed run stayed alive until the watchdog (the window was open)"
      elif printf '%s' "$out" | grep -q "appimage-marker-ok"; then
        pass "windowed run produced the program marker"
      else
        fail "windowed run exited early (status $status): $out"
      fi
    fi
  fi
else
  echo "skip: GUI window case (pass --gui when a display is available)"
fi

if [ "$failures" -ne 0 ]; then
  echo "Linux AppImage runtime tests failed: $failures" >&2
  exit 1
fi
echo "Linux AppImage runtime tests passed"

#!/usr/bin/env bash
# Runtime acceptance for Linux app mode (plan-51-D §4.2, extended by plan-56-C).
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
# Usage: scripts/test-appimage.sh <mfb-exe> [--box <port>] [--libc <l>] [--gui]
#
#   --box <port>  ssh port of the target box. Defaults follow --libc.
#   --libc <l>    glibc | musl | both (default). Selects which AppImage to ship
#                 and which DT_NEEDED assertions apply.
#   --gui         additionally run the windowed case, which needs a real display
#                 on the box. Opt-in, like MFB_MACAPP_GUI=1.
#
# ⚠️ THE LAUNCH PROVES LIVENESS, NOT CORRECTNESS. musl's loader absorbs the
# glibc compat sonames (libc.so.6 and libpthread.so.0 both resolve to ld-musl),
# so a musl AppImage wrongly linked against glibc RUNS PERFECTLY on stock Alpine
# — verified on 2227 and 2224 with gcompat removed and no /lib/libc.so.6 on
# disk. The only thing that distinguishes a correct build from a broken one is
# the inner ELF's DT_NEEDED, which is what `assert_dt_needed` checks. Never
# weaken that case into a smoke test.
set -u

MFB_EXE=${1:-}
if [ -z "$MFB_EXE" ]; then
  echo "usage: test-appimage.sh <mfb-exe> [--box <port>] [--libc <l>] [--gui]" >&2
  exit 2
fi
shift

BOX_OVERRIDE=""
LIBC=both
GUI=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --box) BOX_OVERRIDE=$2; shift 2 ;;
    --libc) LIBC=$2; shift 2 ;;
    --gui) GUI=1; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done
case "$LIBC" in
  glibc|musl|both) ;;
  *) echo "--libc must be glibc|musl|both" >&2; exit 2 ;;
esac

# The measured (arch × libc) box matrix (plan-56-C §4.2.1). **Re-probe rather
# than assume** — three of these facts changed during plan-56 itself:
#   2228  x86_64  glibc   GTK4, /dev/fuse, suid fusermount3  -> FUSE mount works
#   2227  x86_64  musl    GTK4, /dev/fuse, suid fusermount3  -> FUSE mount works
#   2224  aarch64 musl    GTK4, NO /dev/fuse                 -> extract-and-run
#   2226  aarch64 glibc   offline at time of writing
box_for_libc() {
  case "$1" in
    glibc) echo 2228 ;;
    musl) echo 2227 ;;
  esac
}
target_for_box() {
  case "$1" in
    2228|2227) echo linux-x86_64 ;;
    2226|2224) echo linux-aarch64 ;;
    *) echo "" ;;
  esac
}

ROOT=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
failures=0
trap 'rm -rf "$work"' EXIT

pass() { echo "ok: $1"; }
fail() { echo "FAIL: $1" >&2; failures=$((failures + 1)); }

# Run a command on the box under a watchdog. A GUI app that fails to start does
# not exit — it hangs — and a hung ssh in an acceptance script is a wedged
# terminal. Prints output; exits 99 on timeout. `-n` keeps ssh off our stdin.
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

# ⚠️ The only check that can detect a wrongly-flavored build (plan-56-A §2.4).
# Asserts on the ABSENCE of every glibc soname, not merely the presence of the
# musl one: the pre-plan-56-A binaries carried BOTH and would satisfy any
# presence-only assertion while being wrong.
assert_dt_needed() {
  local ssh=$1 elf=$2 libc=$3 label=$4
  local needed
  needed=$($ssh -n "readelf -d '$elf' 2>/dev/null | awk '/NEEDED/{print \$NF}' | tr -d '[]'")
  if [ -z "$needed" ]; then
    fail "$label: could not read DT_NEEDED (is readelf installed on the box?)"
    return
  fi
  local bad=""
  if [ "$libc" = musl ]; then
    for glibc_only in "libc.so.6" "libpthread.so.0" "libdl.so.2" "librt.so.1" "libm.so.6"; do
      if printf '%s\n' "$needed" | grep -qx "$glibc_only"; then
        bad="$bad $glibc_only"
      fi
    done
    if ! printf '%s\n' "$needed" | grep -qE '^libc\.musl-.*\.so\.1$'; then
      bad="$bad (no libc.musl-*.so.1)"
    fi
  else
    if printf '%s\n' "$needed" | grep -qE '^libc\.musl-'; then
      bad="$bad (musl libc in a glibc build)"
    fi
    if ! printf '%s\n' "$needed" | grep -qx "libc.so.6"; then
      bad="$bad (no libc.so.6)"
    fi
  fi
  if [ -n "$bad" ]; then
    fail "$label: wrong DT_NEEDED —$bad
      got: $(printf '%s' "$needed" | tr '\n' ' ')"
  else
    pass "$label: DT_NEEDED names only the $libc world"
  fi
}

# Everything below runs once per selected libc flavor.
run_flavor() {
  local libc=$1
  local port target
  port=${BOX_OVERRIDE:-$(box_for_libc "$libc")}
  target=$(target_for_box "$port")
  if [ -z "$target" ]; then
    fail "unknown box port $port"
    return
  fi

  local ssh="ssh -o ConnectTimeout=8 -o BatchMode=yes -p $port test@127.0.0.1"
  local scp="scp -q -o ConnectTimeout=8 -o BatchMode=yes -P $port"
  if ! $ssh true 2>/dev/null; then
    # The GTK boxes are developer infrastructure, not CI. A suite that goes red
    # for infrastructure reasons trains people to ignore red bars — but a
    # REACHABLE box that fails is a failure.
    echo "skip: $libc box on port $port is not reachable" >&2
    return
  fi
  echo "--- $libc on box $port ($target) ---"

  local remote="/tmp/mfb-appimage-$libc-$$"
  $ssh "mkdir -p $remote" || { fail "$libc: mkdir on box"; return; }

  # A FUSE mount needs BOTH /dev/fuse and a suid fusermount3, and the two fail
  # independently: 2224 has the binary but not the device; 2227 was in the
  # mirror-image state earlier the same day. Probe both, never infer one from
  # the other, and never report a missing mount path as a product failure.
  local run_flags=""
  if $ssh -n 'test -e /dev/fuse && command -v fusermount3 >/dev/null' 2>/dev/null; then
    echo "    (FUSE mount available)"
  else
    run_flags="--appimage-extract-and-run"
    echo "    (no FUSE on this box; using --appimage-extract-and-run)"
  fi

  local proj="$work/$libc"
  mkdir -p "$proj/src"
  cat > "$proj/project.json" <<JSON
{ "name": "hello", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
  printf 'IMPORT io\n\nSUB main()\n  io::print("appimage-marker-ok")\nEND SUB\n' \
    > "$proj/src/main.mfb"
  if ! "$MFB_EXE" build -q --app-debug -target "$target" "$proj" >/dev/null 2>&1; then
    fail "$libc: build --app-debug -target $target"
    return
  fi
  local image="$proj/build/hello-$libc.AppImage"
  local appdir="$proj/build/hello-$libc.AppDir"
  if [ ! -f "$image" ]; then
    fail "$libc: expected $image"
    return
  fi

  $scp "$image" "test@127.0.0.1:$remote/hello.AppImage" || fail "$libc: scp"

  local mode
  mode=$(timeout_run 15 $ssh -n "stat -c %a $remote/hello.AppImage" | tr -d '\r\n')
  if [ "$mode" = "755" ]; then
    pass "$libc: the shipped AppImage is mode 0755"
  else
    fail "$libc: expected mode 755, got '$mode'"
  fi

  # Extract the payload — both the no-FUSE path and how we reach the inner ELF.
  timeout_run 30 $ssh -n "cd $remote && rm -rf root && mkdir root && cd root && \
    ../hello.AppImage --appimage-extract >/dev/null 2>&1; echo done" >/dev/null
  if $ssh -n "test -f $remote/root/squashfs-root/usr/bin/hello"; then
    pass "$libc: --appimage-extract reproduced the payload"
  else
    fail "$libc: --appimage-extract produced no inner executable"
    return
  fi

  # ⚠️ THE correctness check. Everything else here is liveness.
  assert_dt_needed "$ssh" "$remote/root/squashfs-root/usr/bin/hello" "$libc" "$libc"

  # The extracted tree must match the --app-debug AppDir exactly. LC_ALL=C
  # because macOS and Linux `sort` disagree on whether .DirIcon precedes AppRun.
  local local_manifest remote_manifest
  local_manifest=$(cd "$appdir" && find . \( -type f -o -type l \) | LC_ALL=C sort | \
    while read -r p; do
      if [ -L "$p" ]; then printf '%s -> %s\n' "$p" "$(readlink "$p")";
      else printf '%s %s\n' "$p" "$(shasum -a 256 "$p" | cut -d' ' -f1)"; fi
    done)
  remote_manifest=$($ssh -n "cd $remote/root/squashfs-root && \
    find . \\( -type f -o -type l \\) | LC_ALL=C sort | while read -r p; do \
      if [ -L \"\$p\" ]; then printf '%s -> %s\\n' \"\$p\" \"\$(readlink \"\$p\")\"; \
      else printf '%s %s\\n' \"\$p\" \"\$(sha256sum \"\$p\" | cut -d' ' -f1)\"; fi; \
    done")
  if [ "$local_manifest" = "$remote_manifest" ]; then
    pass "$libc: extracted tree matches the --app-debug AppDir exactly"
  else
    fail "$libc: extracted tree differs from the AppDir:
$(diff <(printf '%s\n' "$local_manifest") <(printf '%s\n' "$remote_manifest") | head -12)"
  fi

  # It starts. ⚠️ Liveness only — see the header. Reaching GTK's display probe
  # requires the mount/extract, AppRun, and the dynamic link to have succeeded.
  local out status
  # shellcheck disable=SC2086
  out=$(timeout_run 20 $ssh -n "cd $remote && ./hello.AppImage $run_flags 2>&1")
  status=$?
  if [ "$status" -eq 99 ]; then
    fail "$libc: hung (watchdog fired) — the mount or the GTK boot wedged"
  elif printf '%s' "$out" | grep -qE "appimage-marker-ok|Failed to open display|Gtk-WARNING"; then
    pass "$libc: started the inner GTK program"
  else
    fail "$libc: never reached the GTK bootstrap (exit $status): $out"
  fi

  if $ssh -n "command -v desktop-file-validate" >/dev/null 2>&1; then
    out=$($ssh -n "desktop-file-validate $remote/root/squashfs-root/hello.desktop 2>&1")
    if [ -z "$out" ]; then
      pass "$libc: desktop-file-validate accepts the .desktop"
    else
      fail "$libc: desktop-file-validate rejected the .desktop: $out"
    fi
  fi

  # The script's own acceptance criterion: it must be able to go RED.
  # ⚠️ A one-byte truncation is NOT a corruption — the image is sector-padded
  # and `bytes_used` excludes the padding, so a short image mounts fine
  # (observed on 2228). Corrupt the squashfs magic instead.
  local runtime_len
  runtime_len=$($ssh -n "cd $remote && python3 -c \"
import struct
b=open('hello.AppImage','rb').read(64)
print(struct.unpack_from('<Q',b,0x28)[0]+struct.unpack_from('<H',b,0x3a)[0]*struct.unpack_from('<H',b,0x3c)[0])\"" | tr -d '\r\n')
  if [ -n "$runtime_len" ]; then
    $ssh -n "cd $remote && cp hello.AppImage broken.AppImage && \
      printf '\\x00' | dd of=broken.AppImage bs=1 seek=$runtime_len count=1 conv=notrunc 2>/dev/null"
    # shellcheck disable=SC2086
    out=$(timeout_run 20 $ssh -n "cd $remote && ./broken.AppImage $run_flags 2>&1")
    if printf '%s' "$out" | grep -qE "appimage-marker-ok|Failed to open display|Gtk-WARNING"; then
      fail "$libc: a corrupted AppImage still ran — this script cannot go red"
    else
      pass "$libc: a corrupted squashfs superblock fails (the script can go red)"
    fi
  fi

  # A vendoring build puts each flavor's OWN libc blob in its image, and not the
  # other's (plan-56-B §4.3).
  local blob_arch=${target#linux-}
  local blob="libsndfile.so.1.0.37-$blob_arch-$libc"
  local other_libc=glibc
  [ "$libc" = glibc ] && other_libc=musl
  local other_blob="libsndfile.so.1.0.37-$blob_arch-$other_libc"
  if [ -f "$ROOT/bindings/libsnd/vendor/$blob" ] &&
     [ -f "$ROOT/bindings/libsnd/vendor/$other_blob" ]; then
    local vproj="$work/vend-$libc"
    mkdir -p "$vproj/src" "$vproj/vendor"
    # BOTH blobs are declared and present, exactly as a real project shipping a
    # Linux app must do now: `--app` emits both flavors, so a manifest covering
    # only one libc legitimately fails to resolve for the other half. That also
    # makes this a real test of the per-flavor routing — each image must end up
    # with its own blob and NOT the other's.
    cp "$ROOT/bindings/libsnd/vendor/$blob" "$vproj/vendor/"
    cp "$ROOT/bindings/libsnd/vendor/$other_blob" "$vproj/vendor/"
    local hash other_hash
    hash=$(shasum -a 256 "$ROOT/bindings/libsnd/vendor/$blob" | cut -d' ' -f1)
    other_hash=$(shasum -a 256 "$ROOT/bindings/libsnd/vendor/$other_blob" | cut -d' ' -f1)
    cat > "$vproj/project.json" <<JSON
{ "name": "vend", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "libraries": { "snd": [
    { "os": "linux", "arch": "$blob_arch", "libc": "$libc",
      "type": "vendor", "source": "$blob", "hash": "$hash" },
    { "os": "linux", "arch": "$blob_arch", "libc": "$other_libc",
      "type": "vendor", "source": "$other_blob", "hash": "$other_hash" } ] },
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
    cat > "$vproj/src/main.mfb" <<'MFB'
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
    if ! "$MFB_EXE" build -q --app -target "$target" "$vproj" >/dev/null 2>&1; then
      fail "$libc: build --app with a vendored library"
    else
      $scp "$vproj/build/vend-$libc.AppImage" "test@127.0.0.1:$remote/vend.AppImage" \
        || fail "$libc: scp vend"
      $ssh -n "cd $remote && rm -rf vroot && mkdir vroot && cd vroot && \
        ../vend.AppImage --appimage-extract >/dev/null 2>&1"
      if $ssh -n "test -f $remote/vroot/squashfs-root/usr/lib/vend-$blob"; then
        pass "$libc: the matching vendored library is inside the image"
      else
        fail "$libc: vend-$blob missing from usr/lib inside the image"
      fi
      if $ssh -n "test -f $remote/vroot/squashfs-root/usr/lib/vend-$other_blob"; then
        fail "$libc: the $other_libc blob leaked into the $libc image (plan-56-B §4.3)"
      else
        pass "$libc: the $other_libc blob is correctly absent"
      fi
      local runpath
      runpath=$($ssh -n "readelf -d $remote/vroot/squashfs-root/usr/bin/vend 2>/dev/null \
        | sed -n 's/.*RUNPATH.*\[\(.*\)\].*/\1/p'" | tr -d '\r\n')
      if [ "$runpath" = '$ORIGIN/../lib' ]; then
        pass "$libc: DT_RUNPATH is \$ORIGIN/../lib"
      else
        fail "$libc: expected DT_RUNPATH \$ORIGIN/../lib, got '$runpath'"
      fi
    fi
  else
    echo "skip: $libc vendor case (need both $blob and $other_blob)"
  fi

  if [ "$GUI" -eq 1 ]; then
    local display
    display=$($ssh -n 'for d in ${MFB_APPIMAGE_DISPLAY:-} :0 :1 :2 :3 :4; do
                         [ -n "$d" ] || continue
                         if timeout 3 env DISPLAY=$d xdpyinfo >/dev/null 2>&1; then
                           echo "$d"; break
                         fi
                       done' 2>/dev/null | tr -d '\r\n')
    if [ -z "$display" ]; then
      echo "skip: $libc GUI case (no reachable X display; set MFB_APPIMAGE_DISPLAY)"
    else
      # shellcheck disable=SC2086
      out=$(timeout_run 20 $ssh -n "cd $remote && DISPLAY=$display ./hello.AppImage $run_flags 2>&1")
      status=$?
      # In windowed mode the watchdog firing is EXPECTED: the finish contract
      # parks the worker in pause() so the window stays open. The inverse of
      # every other case here.
      if [ "$status" -eq 99 ]; then
        pass "$libc: windowed run stayed alive until the watchdog (window open)"
      elif printf '%s' "$out" | grep -q "appimage-marker-ok"; then
        pass "$libc: windowed run produced the program marker"
      else
        fail "$libc: windowed run exited early (status $status): $out"
      fi
    fi
  fi

  $ssh "rm -rf $remote" >/dev/null 2>&1 || true
}

if [ "$LIBC" = both ]; then
  run_flavor glibc
  run_flavor musl
else
  run_flavor "$LIBC"
fi

if [ "$failures" -ne 0 ]; then
  echo "Linux AppImage runtime tests failed: $failures" >&2
  exit 1
fi
echo "Linux AppImage runtime tests passed"

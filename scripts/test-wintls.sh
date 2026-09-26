#!/usr/bin/env bash
# Runtime acceptance for the Windows Schannel `tls` backend (bug-721, bug-564).
#
# Every `tests/rt-behavior/tls/*-rt` fixture is a real TLS client against a public
# host, and each one's `golden/build.log` ends with the program's expected output.
# `test-accept.sh` runs them on the host it lives on; nothing ran them on Windows,
# which is how two Schannel memory bugs shipped:
#
#   * bug-564 / bug-721 — every successful `tls::connect` freed its 64 KiB
#     server-name buffer twice (a helper's hand-numbered `%v9` WAS the connect's
#     minted host-scratch pointer). The block sat in a large bin and a quick bin at
#     once; the next small allocation handed it out and the next walk of that bin
#     faulted: `tls-poll-rt` (two connects in one process) never printed `loop=`,
#     and `examples/wind` died with 0xC0000005 while its worker decoded forecasts.
#
# So this cross-builds each fixture for windows-x86_64, ships the executable to the
# Windows box, runs it there and compares its stdout and exit code with the
# `$ <exe>` … `[exit N]` tail of the fixture's `golden/build.log`.
#
# Usage: scripts/test-wintls.sh <mfb-exe> [--box <port>]
#
#   --box <port>   ssh port of the Windows box (default 2230). It needs outbound
#                  HTTPS (the fixtures talk to 8.8.8.8 / public hosts).
set -euo pipefail
. "$(dirname "$0")/remote-common.sh"

MFB_EXE="${1:?usage: test-wintls.sh <mfb-exe> [--box <port>]}"
shift || true
PORT=2230
rc_parse_box test-wintls "$@"
MFB_EXE="$(cd "$(dirname "$MFB_EXE")" && pwd)/$(basename "$MFB_EXE")"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

host="test@127.0.0.1"
remote='C:\mfbtls'
rc_workdir

for fixture in "$ROOT"/tests/rt-behavior/tls/*-rt; do
  name="$(basename "$fixture")"
  golden="$fixture/golden/build.log"
  [ -f "$golden" ] || { fail "$name: no golden/build.log"; continue; }
  # The expected run: everything after the `$ …/build/<exe>` line of the golden.
  # A fixture whose peer is the `openssl` CLI (a local `tls::listen` + `s_client`)
  # cannot run on the Windows box, which has no openssl; say so rather than fail.
  if grep -q '"openssl"' "$fixture"/src/*.mfb; then
    echo "skip: $name (its peer is the openssl CLI, which the Windows box does not have)"
    continue
  fi
  expected="$(awk '/^\$ .*\/build\/[^ ]+$/ { run = 1; next } run' "$golden")"
  [ -n "$expected" ] || { fail "$name: golden has no run section"; continue; }

  # Build a copy so the fixture directory stays clean.
  copy="$work/$name"
  cp -R "$fixture" "$copy"
  rm -rf "$copy/build" "$copy/golden"
  if ! "$MFB_EXE" build -q -target windows-x86_64 "$copy" >"$work/$name.build" 2>&1; then
    fail "$name: build failed: $(tr '\n' ' ' <"$work/$name.build")"
    continue
  fi
  exe="$(find "$copy/build" -maxdepth 1 -name '*.exe' | head -n 1)"
  [ -n "$exe" ] || { fail "$name: no .exe produced"; continue; }

  win_ship "$PORT" "$host" "$remote" "$exe"
  exe_name="$(basename "$exe")"
  # `echo [exit %errorlevel%]` reproduces the golden's trailer.
  printf '@echo off\r\ncd /d %s\r\n%s\r\necho [exit %%errorlevel%%]\r\n' "$remote" "$exe_name" \
    >"$work/run-$name.bat"
  win_ship "$PORT" "$host" "$remote" "$work/run-$name.bat"
  actual="$(watchdog 180 ssh $RC_SSH_OPTS -p "$PORT" "$host" "$remote\\run-$name.bat" 2>/dev/null \
    | tr -d '\r' | grep -v '^The system cannot find the path specified\.$' || true)"

  if [ "$actual" = "$expected" ]; then
    pass "$name"
  else
    fail "$name: output differs"
    diff <(printf '%s\n' "$expected") <(printf '%s\n' "$actual") | sed 's/^/    /' || true
  fi
done

if [ "$rc_failures" -ne 0 ]; then
  echo "test-wintls: $rc_failures failure(s)"
  exit 1
fi
echo "test-wintls: all fixtures match"

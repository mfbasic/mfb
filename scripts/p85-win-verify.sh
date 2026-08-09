#!/usr/bin/env bash
# plan-85 Win64 execution verification: compile each fixture for windows-x86_64, ship
# the .exe to box 2230 (Win11), run it (output redirected to a file — MFB writes via
# WriteFile, which redirects), read it back, and diff stdout against the golden's
# execution section (between the final `$ ...out`/`.exe` line and its `[exit ...]`).
# usage: p85-win-verify.sh <fixture-dir>...
set -u
EXE=target/release/mfb
PORT=2230
pass=0; fail=0
for dir in "$@"; do
  log="$dir/golden/build.log"
  [ -f "$log" ] || { echo "SKIP  $dir (no build.log)"; continue; }
  expected=$(awk '/^\$ .*(\.out|\.exe)$/{cap=1; buf=""; next} /^\[exit /{if(cap){printf "%s",buf; cap=0}} cap{buf=buf $0 "\n"}' "$log")
  rm -rf "$dir/build" 2>/dev/null
  outline=$(MFB_TARGET=windows-x86_64 "$EXE" build -q -target windows-x86_64 "$dir" 2>/dev/null | sed -n 's/^Wrote executable to //p' | grep -- '.exe' | head -1)
  [ -n "$outline" ] || { echo "FAIL  $dir (no .exe)"; fail=$((fail+1)); continue; }
  scp -q -P "$PORT" -o StrictHostKeyChecking=no -o ConnectTimeout=15 "$outline" test@127.0.0.1:p85run.exe 2>/dev/null
  ssh -p "$PORT" -o StrictHostKeyChecking=no -o ConnectTimeout=15 test@127.0.0.1 'cmd /c "p85run.exe > p85run.txt 2>&1"' >/dev/null 2>&1
  actual=$(ssh -p "$PORT" -o StrictHostKeyChecking=no -o ConnectTimeout=15 test@127.0.0.1 'cmd /c "type p85run.txt"' 2>/dev/null | tr -d '\r')
  rm -rf "$dir/build" 2>/dev/null
  if [ "$actual" = "$expected" ]; then
    echo "PASS  $dir"; pass=$((pass+1))
  else
    echo "FAIL  $dir"; fail=$((fail+1))
    echo "--- expected ---"; printf '%s\n' "$expected" | head -6
    echo "--- actual ---";   printf '%s\n' "$actual" | head -6
    echo "----------------"
  fi
done
echo "=== p85 Win64 verify: pass=$pass fail=$fail ==="

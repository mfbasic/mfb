#!/usr/bin/env bash
# plan-85 SysV-x86 execution verification: compile each fixture for linux-x86_64,
# ship the glibc binary to box 2228, run it, and diff stdout against the golden's
# execution section (the lines between the final `$ ...out` and its `[exit ...]`).
# usage: p85-sysv-verify.sh <fixture-dir>...
set -u
EXE=target/release/mfb
PORT=2228
pass=0; fail=0
for dir in "$@"; do
  # Extract the expected execution stdout from golden/build.log.
  log="$dir/golden/build.log"
  [ -f "$log" ] || { echo "SKIP  $dir (no build.log)"; continue; }
  expected=$(awk '/^\$ .*\.out$/{cap=1; buf=""; next} /^\[exit /{if(cap){printf "%s",buf; cap=0}} cap{buf=buf $0 "\n"}' "$log")
  # Compile for linux-x86_64.
  outline=$(MFB_TARGET=linux-x86_64 "$EXE" build -q -target linux-x86_64 "$dir" 2>/dev/null | sed -n 's/^Wrote executable to //p' | grep -- '-glibc.out' | head -1)
  [ -n "$outline" ] || { echo "FAIL  $dir (compile produced no glibc.out)"; fail=$((fail+1)); continue; }
  scp -q -P "$PORT" -o StrictHostKeyChecking=no "$outline" test@127.0.0.1:/tmp/p85run.out 2>/dev/null
  actual=$(ssh -p "$PORT" -o StrictHostKeyChecking=no test@127.0.0.1 'chmod +x /tmp/p85run.out && /tmp/p85run.out 2>&1' 2>/dev/null)
  rm -rf "$dir/build" 2>/dev/null
  if [ "$actual" = "$expected" ]; then
    echo "PASS  $dir"; pass=$((pass+1))
  else
    echo "FAIL  $dir"; fail=$((fail+1))
    echo "----- expected -----"; printf '%s\n' "$expected" | head -8
    echo "----- actual -------"; printf '%s\n' "$actual" | head -8
    echo "--------------------"
  fi
done
echo "=== p85 SysV verify: pass=$pass fail=$fail ==="

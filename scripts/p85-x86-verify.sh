#!/usr/bin/env bash
# plan-85-D SysV-x86 EXECUTION verify: compile each fixture for linux-x86_64, ship the
# musl ELF to box 2227 (Alpine x86_64), run it, and diff stdout against the golden's
# execution section (between the final `$ .../<pkg>.out` line and its `[exit ...]`).
# SysV-x86 is byte-CHANGING under plan-85's aligned ABI, so execution is the check.
# usage: p85-x86-verify.sh <fixture-dir>...
set -u
EXE=target/release/mfb
PORT=2227
pass=0; fail=0
for dir in "$@"; do
  log="$dir/golden/build.log"
  [ -f "$log" ] || { echo "SKIP  $dir (no build.log)"; continue; }
  expected=$(awk '/^\$ .*\.out$/{cap=1; buf=""; next} /^\[exit /{if(cap){printf "%s",buf; cap=0}} cap{buf=buf $0 "\n"}' "$log")
  rm -rf "$dir/build" 2>/dev/null
  out=$("$EXE" build -q -target linux-x86_64 "$dir" 2>/dev/null | sed -n 's/^Wrote executable to //p' | grep -- '-musl.out' | head -1)
  [ -n "$out" ] || { echo "FAIL  $dir (no elf)"; fail=$((fail+1)); continue; }
  scp -q -P "$PORT" -o StrictHostKeyChecking=no -o ConnectTimeout=15 "$out" test@127.0.0.1:p85run 2>/dev/null
  actual=$(ssh -p "$PORT" -o StrictHostKeyChecking=no -o ConnectTimeout=20 test@127.0.0.1 'chmod +x p85run; ./p85run 2>&1' 2>/dev/null)
  rm -rf "$dir/build" 2>/dev/null
  if [ "$actual" = "$expected" ]; then
    echo "PASS  $dir"; pass=$((pass+1))
  else
    echo "FAIL  $dir"; fail=$((fail+1))
    echo "--- expected ---"; printf '%s\n' "$expected" | head -6
    echo "--- actual ---";   printf '%s\n' "$actual" | head -6
  fi
done
echo "=== p85 SysV-x86 verify: pass=$pass fail=$fail ==="

#!/usr/bin/env bash
# For each `compare_immediate(abi::c_return(0)` in win_x86_64/code.rs, report whether a
# Win32 call (call_external / branch_link to an ALLCAPS import / emit_variadic_call /
# emit_libc_call) appears in the preceding 7 lines. "call_before=0" flags a compare that
# is likely checking an MFB value (a prior helper's return), where the sed was WRONG.
set -u
cd /Users/justinzaun/Development/mfb/.claude/worktrees/P-85 || exit 2
F=src/target/win_x86_64/code.rs
for ln in $(grep -n 'compare_immediate(abi::c_return(0)' "$F" | cut -d: -f1); do
  start=$((ln-13))
  has=$(awk -v a="$start" -v b="$ln" 'NR>=a && NR<b && (/call_external\(/ || /emit_variadic_call/ || /emit_libc_call/ || /branch_link\("[A-Z]/){f=1} END{print f+0}' "$F")
  echo "line $ln call_before=$has"
done

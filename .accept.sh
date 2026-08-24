#!/usr/bin/env bash
# Wait for the peer session's test-accept to clear (the concurrency guard matches
# across worktrees), then run this worktree's acceptance harness once.
cd /Users/justinzaun/Development/mfb/.claude/worktrees/P-102 || exit 3
waited=0
while pgrep -f 'test-accept.sh' | grep -v $$ >/dev/null 2>&1; do
  sleep 30
  waited=$((waited + 30))
  if [ "$waited" -ge 2400 ]; then echo "TIMEOUT waiting for peer test-accept"; exit 1; fi
done
echo "peer clear after ${waited}s; running acceptance..."
scripts/test-accept.sh target/release/mfb /tmp/accept-out 2>&1 | tail -20
echo "accept-exit=$?"

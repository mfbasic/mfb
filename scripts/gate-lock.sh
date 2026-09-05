#!/usr/bin/env bash
# bug-470: mutual exclusion between `artifact-gate.sh` and `test-accept.sh`.
#
# WHAT THEY CONTEND FOR. Both regenerate and delete the SAME fixture dump files
# under the tree they run in. Two of them in one tree corrupt each other's
# artifacts — 0-byte dumps, "expected vs actual" failures on fixtures neither
# run touched. So within a tree they must be exclusive, in BOTH orders.
#
# WHAT THEY DO NOT CONTEND FOR. Each worktree owns its own `tests/`. Two runs in
# two different trees cannot corrupt each other, and must be allowed to proceed
# concurrently — a machine with nine worktrees is the normal case here, not an
# exotic one.
#
# The guards this replaces got BOTH halves wrong, from one missing dimension:
# they keyed on the script's NAME and never on which tree it belonged to.
#
#   * too narrow — each `pgrep -f` pattern matched only its own script, so an
#     artifact-gate and a test-accept in ONE tree never saw each other. (The
#     filed bug.)
#   * too broad — `*/artifact-gate.sh` matches any path, so a run in
#     worktrees/467 refused a run in worktrees/474. Pure lost throughput.
#
# WHY A LOCK FILE AND NOT A WIDENED `pgrep`. The old guards were check-then-act:
# `pgrep`, then proceed. A run that observes a free lock has learned something
# about the past, not made a claim about the present, so two runs that check at
# the same moment both proceed. With ~8 concurrent writers that window is
# sampled continuously. `mkdir` is atomic on every filesystem we target and
# needs no `flock(1)`, which macOS does not ship.
#
# The lock lives in the TREE, so it is per-tree by construction rather than by a
# path comparison someone can get wrong later.
#
# Callers set `GATE_LOCK_HOLDER` (their own script name, for the refusal
# message) and may set `GATE_LOCK_TREE` (defaults to the tree this helper lives
# in — derived from the helper's own path, never `$PWD`, since `$PWD` is
# whatever directory the operator happened to be in).

# Tree this run belongs to: the parent of the directory holding this helper.
if [ -z "${GATE_LOCK_TREE:-}" ]; then
  GATE_LOCK_TREE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi
GATE_LOCK_DIR="$GATE_LOCK_TREE/tests/.gate.lock"
GATE_LOCK_HELD=""

# Exit 98, not 1: a refusal is NOT a gate result. Sharing 1 with "found diffs"
# makes a lock collision read as a golden regression, and the reader spends
# their time on the wrong question. `tests/golden.rs` branches on 98 to report
# "nothing was checked".
GATE_LOCK_REFUSED=98

gate_lock_release() {
  # Only the holder releases. Guards against an inherited trap in a subshell
  # tearing down a lock this shell never took.
  if [ -n "$GATE_LOCK_HELD" ]; then
    rm -rf "$GATE_LOCK_DIR"
    GATE_LOCK_HELD=""
    unset MFB_GATE_LOCK_OWNER
  fi
}

# True when `pid` is not a live process, i.e. the lock is a leftover from a run
# that was killed before its EXIT trap could fire. Without this, one `^C` at the
# wrong moment wedges the tree until someone deletes the directory by hand.
gate_lock_holder_is_gone() {
  local pid="$1"
  [ -z "$pid" ] && return 0
  kill -0 "$pid" 2>/dev/null && return 1
  return 0
}

gate_lock_acquire() {
  local holder="${GATE_LOCK_HOLDER:-gate}"
  mkdir -p "$(dirname "$GATE_LOCK_DIR")" 2>/dev/null

  # RE-ENTRANCY. `sync-goldens.sh` must hold the lock across BOTH the
  # `test-accept.sh` run it spawns AND the golden copy that follows it —
  # otherwise the copy happens unlocked and an `artifact-gate` starting in that
  # window reads half-written goldens, which is the corruption this lock exists
  # to prevent. But the `test-accept.sh` it spawns calls this function too, and
  # would refuse its own parent. So an acquire is a no-op when this process
  # tree already holds THIS tree's lock: the owner pid is exported, children
  # inherit it, and a child neither takes nor releases. The pid is re-checked
  # against the live lock so a stale exported value from an earlier run in the
  # same shell cannot wave a caller through.
  if [ -n "${MFB_GATE_LOCK_OWNER:-}" ] && [ -d "$GATE_LOCK_DIR" ]; then
    local live_pid
    live_pid=$(awk '{print $2}' "$GATE_LOCK_DIR/owner" 2>/dev/null)
    if [ "$live_pid" = "$MFB_GATE_LOCK_OWNER" ]; then
      return 0
    fi
  fi

  if mkdir "$GATE_LOCK_DIR" 2>/dev/null; then
    printf '%s %s %s\n' "$holder" "$$" "$(date +%s 2>/dev/null || echo 0)" \
      > "$GATE_LOCK_DIR/owner"
    GATE_LOCK_HELD=1
    export MFB_GATE_LOCK_OWNER=$$
    trap gate_lock_release EXIT INT TERM
    return 0
  fi

  # Held — by a live run, or left behind by a dead one.
  local owner_script owner_pid
  owner_script=$(awk '{print $1}' "$GATE_LOCK_DIR/owner" 2>/dev/null)
  owner_pid=$(awk '{print $2}' "$GATE_LOCK_DIR/owner" 2>/dev/null)

  if gate_lock_holder_is_gone "$owner_pid"; then
    # Reclaim, then retry ONCE. A second failure means a live rival won the
    # race to reclaim it, which is a legitimate refusal.
    rm -rf "$GATE_LOCK_DIR"
    if mkdir "$GATE_LOCK_DIR" 2>/dev/null; then
      printf '%s %s %s\n' "$holder" "$$" "$(date +%s 2>/dev/null || echo 0)" \
        > "$GATE_LOCK_DIR/owner"
      GATE_LOCK_HELD=1
      export MFB_GATE_LOCK_OWNER=$$
      trap gate_lock_release EXIT INT TERM
      return 0
    fi
    owner_script=$(awk '{print $1}' "$GATE_LOCK_DIR/owner" 2>/dev/null)
    owner_pid=$(awk '{print $2}' "$GATE_LOCK_DIR/owner" 2>/dev/null)
  fi

  # Name the rival and the tree. The old message said only "another
  # artifact-gate is running", which was actively misleading once the guard
  # could refuse across trees — the operator would look in the wrong checkout.
  echo "Refusing to run: ${owner_script:-another gate} (pid ${owner_pid:-?}) holds $GATE_LOCK_DIR" >&2
  echo "  Both scripts rewrite the same fixture dumps in this tree." >&2
  echo "  A run in a DIFFERENT worktree does not conflict and is not refused." >&2
  return "$GATE_LOCK_REFUSED"
}

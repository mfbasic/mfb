#!/usr/bin/env bash
# The full tier: the whole cargo suite, sweeps included, at one commit, in its own
# checkout, so it never blocks the tree a person or agent is landing from.
#
# It makes a detached worktree of <rev> under $MFB_FULL_ROOT/trees/, runs
# `cargo test --workspace --no-fail-fast` there, and records the result per commit
# in $MFB_FULL_ROOT/runs/. Landing does not wait for it: run it in the background
# on main's tip (nightly, or after a batch of merges). A red result is a bug to file,
# then bisect. `.ai/testing-gates.md` "Test tiers" is the policy.
#
# ONE AT A TIME PER MACHINE. Two full runs at once each take about twice as long
# (the same unit binary measured 697 s alone and 2,467 s under a peer run), so a
# second run refuses with exit 98 while a live one holds $MFB_FULL_ROOT/lock.
#
# Fresh per run: the worktree (so no gitignored fixture cache, e.g. a stale
# `build/packages/*.mfp`, leaks in from a working tree). Shared across runs:
# CARGO_TARGET_DIR=$MFB_FULL_ROOT/target, so each run rebuilds only what changed.
# On success the worktree is removed; on failure it is kept for inspection, and
# the next run of the same commit replaces it.
set -u

usage() {
  cat >&2 <<'EOF'
Usage: bash scripts/test-full.sh [--with-ignored] [<rev>] [-- <extra cargo test args>]

  <rev>           commit to test (default: HEAD of this checkout)
  --with-ignored  also run rt_inplace_self_update's #[ignore]d grid (hours)
  -- <args>       appended to `cargo test --workspace --no-fail-fast`, e.g.
                  `-- --test rt_canvas_tile` to re-run one binary at that commit

Env: MFB_FULL_ROOT (default: ${XDG_CACHE_HOME:-$HOME/.cache}/mfb-full-suite)
Exit: cargo's status; 98 when another full run holds the lock; 2 on bad usage.
EOF
}

REPO="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="${MFB_FULL_ROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/mfb-full-suite}"

REV=HEAD
WITH_IGNORED=0
EXTRA=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --with-ignored) WITH_IGNORED=1; shift ;;
    --) shift; EXTRA=("$@"); break ;;
    -h|--help) usage; exit 0 ;;
    -*) echo "error: unknown option '$1'" >&2; usage; exit 2 ;;
    *) REV="$1"; shift ;;
  esac
done

SHA="$(git -C "$REPO" rev-parse --verify --quiet "$REV^{commit}")"
if [ -z "$SHA" ]; then
  echo "error: '$REV' is not a commit" >&2
  exit 2
fi
SHORT="${SHA:0:12}"

mkdir -p "$ROOT/runs" "$ROOT/trees" || exit 2

# The machine-wide lock. `mkdir` is atomic; a lock whose owner pid is dead is a
# leftover from a killed run and is reclaimed once.
LOCK="$ROOT/lock"
take_lock() {
  if mkdir "$LOCK" 2>/dev/null; then
    echo "$$ $SHORT $(date +%s)" > "$LOCK/owner"
    return 0
  fi
  return 1
}
if ! take_lock; then
  owner_pid=$(awk '{print $1}' "$LOCK/owner" 2>/dev/null)
  if [ -n "$owner_pid" ] && kill -0 "$owner_pid" 2>/dev/null; then
    echo "Refusing to run: a full run (pid $owner_pid, $(awk '{print $2}' "$LOCK/owner")) holds $LOCK" >&2
    exit 98
  fi
  rm -rf "$LOCK"
  if ! take_lock; then
    echo "Refusing to run: another full run took $LOCK first" >&2
    exit 98
  fi
fi
trap 'rm -rf "$LOCK"' EXIT INT TERM

TREE="$ROOT/trees/$SHORT"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG="$ROOT/runs/$SHORT-$STAMP.log"
RESULT="$ROOT/runs/$SHORT.result"

# A tree left by an earlier failed run of this commit is replaced, not reused.
if [ -e "$TREE" ]; then
  git -C "$REPO" worktree remove --force "$TREE" 2>/dev/null || rm -rf "$TREE"
  git -C "$REPO" worktree prune
fi
if ! git -C "$REPO" worktree add --detach "$TREE" "$SHA" > "$LOG" 2>&1; then
  echo "error: could not create the worktree; see $LOG" >&2
  exit 2
fi

echo "test-full: $SHORT in $TREE"
echo "test-full: log $LOG"
START=$(date +%s)
(
  cd "$TREE" || exit 2
  export CARGO_TARGET_DIR="$ROOT/target"
  cargo test --workspace --no-fail-fast "${EXTRA[@]+"${EXTRA[@]}"}"
) >> "$LOG" 2>&1
RC=$?
IGNORED_RC=skipped
if [ "$WITH_IGNORED" -eq 1 ]; then
  (
    cd "$TREE" || exit 2
    export CARGO_TARGET_DIR="$ROOT/target"
    cargo test --test rt_inplace_self_update -- --ignored
  ) >> "$LOG" 2>&1
  IGNORED_RC=$?
  if [ "$RC" -eq 0 ] && [ "$IGNORED_RC" -ne 0 ]; then
    RC="$IGNORED_RC"
  fi
fi
END=$(date +%s)

# Per-commit record. The newest run of a commit replaces the older record; every
# run's log is kept.
{
  echo "sha $SHA"
  echo "started $(date -r "$START" '+%Y-%m-%d %H:%M:%S' 2>/dev/null || echo "$START")"
  echo "seconds $((END - START))"
  echo "exit $RC"
  echo "ignored_grid $IGNORED_RC"
  echo "log $LOG"
  echo "binaries_reached $(grep -cE '^ +Running ' "$LOG")"
  echo "failing_binaries:"
  grep -E '^error: test failed, to rerun pass' "$LOG" | sed 's/^/  /'
  echo "failing_tests:"
  grep -E '^test .+ \.\.\. FAILED$' "$LOG" | sed -E 's/^test (.+) \.\.\. FAILED$/  \1/' | sort -u
} > "$RESULT"

if [ "$RC" -eq 0 ]; then
  git -C "$REPO" worktree remove --force "$TREE"
  echo "test-full: $SHORT ok ($((END - START)) s). Record: $RESULT"
else
  echo "test-full: $SHORT FAILED (exit $RC). Record: $RESULT; tree kept at $TREE"
fi
exit "$RC"

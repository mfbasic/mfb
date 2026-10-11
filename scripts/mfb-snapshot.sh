#!/usr/bin/env bash
# Run a harness against a private COPY of the `mfb` binary it was handed.
#
# WHY. `artifact-gate.sh` and `test-accept.sh` invoke the compiler once per
# fixture, for minutes. If `target/release/mfb` is rebuilt during that time the
# run's fixtures get built by two different compilers: phantom `[exit 139]`s,
# goldens that "change again", all gone on a clean rerun
# (`.ai/testing-gates.md`, "Concurrency & macOS hazards"). A rebuild needs no
# source edit to happen — `build.rs` watches `.git/HEAD` and `.git/index` for the
# `mfb --version` commit stamp, so a commit or a `git add` makes the next
# `cargo build` relink the binary.
#
# A snapshot taken once, before the first fixture, makes the run immune: it
# tests the compiler that existed when it started, whatever happens to the
# original afterwards.
#
# Sourced, not executable. `mfb_snapshot_take <exe>` sets:
#   MFB_SNAPSHOT_DIR   the private directory (the caller's EXIT trap removes it)
#   MFB_SNAPSHOT_EXE   the copy to run, an absolute path
#
# The caller owns the cleanup because `trap` replaces rather than chains
# (bug-470): a trap installed here would silently drop the caller's gate-lock
# release.

MFB_SNAPSHOT_DIR=""
MFB_SNAPSHOT_EXE=""

mfb_snapshot_take() {
  local src="$1" who="${GATE_LOCK_HOLDER:-mfb-snapshot}"

  if [ ! -f "$src" ] || [ ! -x "$src" ]; then
    echo "$who: mfb executable not found or not executable: $src" >&2
    return 2
  fi

  MFB_SNAPSHOT_DIR=$(mktemp -d) || {
    echo "$who: cannot create a directory for the mfb snapshot" >&2
    return 2
  }
  # The copy keeps the name `mfb`, so anything that prints the program's own
  # name reads the same from the snapshot as from the original.
  MFB_SNAPSHOT_EXE="$MFB_SNAPSHOT_DIR/mfb"

  # A copy that does not match its source was taken while the source was being
  # replaced. Refuse it: a run driven by a torn binary is the failure this
  # helper exists to prevent.
  if ! cp "$src" "$MFB_SNAPSHOT_EXE" || ! cmp -s "$src" "$MFB_SNAPSHOT_EXE"; then
    echo "$who: $src changed while it was being copied (is a build running?)" >&2
    rm -rf "$MFB_SNAPSHOT_DIR"
    MFB_SNAPSHOT_DIR=""
    MFB_SNAPSHOT_EXE=""
    return 2
  fi
  return 0
}

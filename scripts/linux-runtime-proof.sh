#!/usr/bin/env bash
# Run the acceptance suite's `.run` fixtures on a REAL Linux box and diff the
# program output against the committed goldens.
#
# Why this exists: none of the Linux boxes carries a Rust toolchain (2229 is the
# lone exception), so `scripts/test-accept.sh` cannot be run there directly. The
# compiler cross-compiles, so this builds every runnable fixture here, ships the
# executable over ssh, runs it on the target hardware, and compares what it
# printed against the `$ <exe>` ... `[exit N]` tail of that fixture's
# `golden/build.log` — the same bytes `test-accept.sh` compares locally.
#
# This is the behavioral half of bug-321's proof. The artifact half
# (`scripts/linux-artifact-baseline.sh`) shows the emitted bytes did not change;
# this shows those bytes still execute correctly on aarch64, x86-64, and riscv64.
#
# Usage:
#   scripts/linux-runtime-proof.sh <mfb-exe> <ssh-port> <target> [flavor]
#     e.g. scripts/linux-runtime-proof.sh target/release/mfb 2223 linux-aarch64 glibc
#   FILTER=<substring> ... restrict to fixtures whose path contains it
#   JOBS=<n>           ... fixtures built+run concurrently (default 4)
set -u

# Repo root. Overridable so the script can be run from a copy outside the tree
# (handy when a long run must not be disturbed by edits to the original — bash
# reads a script incrementally, so editing a running one corrupts it mid-run).
ROOT=${MFB_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
MFB=${1:?usage: linux-runtime-proof.sh <mfb-exe> <ssh-port> <target> [flavor]}
PORT=${2:?usage: linux-runtime-proof.sh <mfb-exe> <ssh-port> <target> [flavor]}
TARGET=${3:?usage: linux-runtime-proof.sh <mfb-exe> <ssh-port> <target> [flavor]}
FLAVOR=${4:-glibc}
MFB=$(cd "$(dirname "$MFB")" && pwd)/$(basename "$MFB")
FILTER=${FILTER:-}
JOBS=${JOBS:-4}
# Per-fixture wall-clock cap. This is a hang detector, not a performance budget,
# so it is generous: the point is to stop a deadlocked fixture from wedging the
# run, not to assert how fast a program should be.
#
# It was 60s, which produced a load-dependent false failure. `crypto/
# crypto-kat-valid` takes ~8s on an idle x86_64/musl box (2227 is qemu TCG on
# Apple Silicon), but at JOBS=10 ten of those contend and it blew past 60s and
# was recorded as a FAIL — a fixture that passes when run alone. A harness that
# fails differently depending on `-P` is worse than a slow one: it teaches you to
# discount its output, which is exactly how the last four failures sat
# unexplained across three sessions.
RUN_TIMEOUT=${RUN_TIMEOUT:-300}
SSH="ssh -o ConnectTimeout=10 -o BatchMode=yes -p $PORT test@127.0.0.1"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/parts"
REMOTE=/tmp/mfb-runtime-proof.$$
$SSH "mkdir -p $REMOTE" || { echo "cannot reach box on port $PORT" >&2; exit 2; }
trap 'rm -rf "$work"; $SSH "rm -rf $REMOTE" >/dev/null 2>&1' EXIT

# Fixtures address their data files by repo-root-relative path (e.g.
# `tests/rt-behavior/fs/.../src/main.mfb`), because `test-accept.sh` runs every
# executable with the repo root as cwd. Mirror that exactly: ship the `tests`
# tree once and run each binary from the directory that contains it. Without
# this, every fs fixture silently reports "not found" and still exits 0 —
# a failure that looks like a real regression but is pure harness error.
echo "shipping tests/ to port $PORT ..."
tar -C "$ROOT" --no-xattrs -cf - tests 2>/dev/null | \
  $SSH "mkdir -p $REMOTE/root && tar -C $REMOTE/root -xf - 2>/dev/null" \
  || { echo "failed to ship tests/ to the box" >&2; exit 2; }

# `target/` is the scratch directory 78 fixtures write into, addressed relative
# to cwd (`fs::writeText("target/bug159_regfile", ...)`). Locally it always
# exists because it is cargo's build directory, so `test-accept.sh` never had to
# create it. Here it does not, and every one of those fixtures failed on the
# write with a "not found" that looked exactly like a product regression — the
# same class of harness error the `cd $REMOTE/root` comment above describes.
#
# One shared directory is correct rather than one per fixture: that is what the
# real harness has. It is also concurrency-safe at any JOBS, because no
# `target/` path is written by more than one fixture (checked across all 78).
$SSH "mkdir -p $REMOTE/root/target" \
  || { echo "failed to create target/ on the box" >&2; exit 2; }

# The expected program output: everything after the LAST `$ <exe>` line in the
# fixture's golden build.log.
#
# Taken verbatim, including the trailing `[exit N]`. That marker is NOT always on
# its own line: `test-accept.sh` appends it with `echo` right after the program's
# output, so a program whose last write has no trailing newline (every `term::`
# fixture) ends up with `...[0m[exit 0]` on one line. This harness reproduces the
# output the same way, so a verbatim tail compares correctly in both shapes —
# an earlier version parsed the marker as its own line and reported every term
# fixture as a failure.
expected_of() {
  awk '
    /^\$ .*\.out$/ { start = NR }
    { line[NR] = $0 }
    END { for (i = start + 1; i <= NR; i++) print line[i] }
  ' "$1"
}
export -f expected_of 2>/dev/null || true

run_fixture() {
  project=$1
  slot=$2
  proj=$(dirname "$project")
  rel=${proj#"$ROOT"/tests/}
  name=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$project" | head -1)
  [ -n "$name" ] || return 0
  [ -f "$proj/golden/$name.run" ] || return 0
  [ -f "$proj/golden/build.log" ] || return 0
  # A fixture whose golden records no `$ <exe>` line is not expected to produce a
  # runnable binary at all (the `*-invalid` build-rejection fixtures keep a
  # `.run` marker but never reach execution). There is nothing to run or compare.
  grep -q '^\$ .*\.out$' "$proj/golden/build.log" || return 0

  part="$WORKDIR/parts/$(printf '%s' "$rel" | shasum -a 256 | cut -c1-32)"
  scratch="$WORKDIR/w$slot"
  rm -rf "$scratch"; cp -R "$proj" "$scratch"; rm -rf "$scratch/build"

  if ! "$MFB" build -q --target "$TARGET" "$scratch" >"$WORKDIR/build$slot.log" 2>&1; then
    echo "BUILD-FAIL|$rel" > "$part"
    return 0
  fi
  exe="$scratch/build/$name-$FLAVOR.out"
  [ -f "$exe" ] || exe="$scratch/build/$name.out"
  if [ ! -f "$exe" ]; then
    echo "NO-EXE|$rel" > "$part"
    return 0
  fi

  # Land the executable where the golden says it lives — `tests/<rel>/build/
  # <name>.out` under the shipped root — and invoke it by exactly that
  # repo-root-relative path.
  #
  # Both halves matter. `test-accept.sh` runs `$run_path` verbatim from the repo
  # root, so a program reading `args` sees that relative path as `argv[0]`, and
  # the golden records it. Running the same bytes from `/tmp/<pid>-<name>`
  # produced a different `argv[0]` and failed
  # `rt-behavior/project/project-entry-args-runtime` on output that was
  # otherwise correct. The `.out` name is deliberately unflavored even when the
  # build emitted `<name>-glibc.out`/`<name>-musl.out`: the goldens are recorded
  # on macOS, which emits one unflavored artifact, and it is `argv[0]` we are
  # matching.
  #
  # Invoked as the bare relative path, with no `./` prefix — the golden has
  # none, and a `./` would land in `argv[0]` and fail the compare just as the
  # absolute path did. A relative path containing a slash executes directly
  # without needing `./`.
  remote_dir="$REMOTE/root/tests/$rel/build"
  remote_rel="tests/$rel/build/$name.out"
  remote_exe="$remote_dir/$name.out"
  $SSH "mkdir -p '$remote_dir'" 2>/dev/null || { echo "SCP-FAIL|$rel" > "$part"; return 0; }
  if ! scp -q -o ConnectTimeout=10 -o BatchMode=yes -P "$PORT" \
        "$exe" "test@127.0.0.1:$remote_exe" 2>/dev/null; then
    echo "SCP-FAIL|$rel" > "$part"
    return 0
  fi

  # cwd is the shipped repo root, exactly as under test-accept.sh.
  actual=$($SSH "cd $REMOTE/root && chmod +x '$remote_rel' && \
      timeout $RUN_TIMEOUT '$remote_rel' </dev/null 2>&1; echo \"[exit \$?]\"" 2>&1)
  expected=$(expected_of "$proj/golden/build.log")

  if [ "$actual" = "$expected" ]; then
    echo "PASS|$rel" > "$part"
  else
    {
      echo "FAIL|$rel"
      echo "--- expected"; printf '%s\n' "$expected"
      echo "--- actual"; printf '%s\n' "$actual"
      echo "---"
    } > "$part"
  fi
}
export -f run_fixture 2>/dev/null || true
export WORKDIR=$work MFB ROOT TARGET FLAVOR PORT SSH REMOTE

: > "$work/projects"
find "$ROOT/tests" -name project.json | sort | while IFS= read -r project; do
  rel=$(dirname "$project"); rel=${rel#"$ROOT"/tests/}
  case "$rel" in
    *"$FILTER"*) printf '%s\n' "$project" >> "$work/projects" ;;
  esac
done

xargs -P "$JOBS" -I{} bash -c 'run_fixture "$1" "$$"' _ {} < "$work/projects"

cat "$work"/parts/* > "$work/results" 2>/dev/null || :
pass=$(grep -c '^PASS|' "$work/results" || true)
fail=$(grep -c '^FAIL|' "$work/results" || true)
other=$(grep -cE '^(BUILD-FAIL|NO-EXE|SCP-FAIL)\|' "$work/results" || true)

# The verdict list, one line per fixture, so the same run against a different
# compiler can be diffed. For a refactor whose contract is "no behavior change",
# an identical verdict list is the proof; a nonzero failure count on its own says
# nothing, because some fixtures fail for reasons that predate the change.
if [ -n "${VERDICTS:-}" ]; then
  grep -oE '^(PASS|FAIL|BUILD-FAIL|NO-EXE|SCP-FAIL)\|.*' "$work/results" | sort > "$VERDICTS"
fi

echo "linux runtime proof: $TARGET/$FLAVOR on port $PORT — $pass passed, $fail failed, $other not run"
if [ "$fail" -gt 0 ] || [ "$other" -gt 0 ]; then
  grep -A 40 -E '^(FAIL|BUILD-FAIL|NO-EXE|SCP-FAIL)\|' "$work/results" | head -200
  exit 1
fi
exit 0

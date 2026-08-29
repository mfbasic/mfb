#!/usr/bin/env bash
# Diagnostic set-equality harness (plan-107-A §3).
#
# A rule relocation (syntaxcheck → ir::verify, or → hir::shape) may change the
# ORDER diagnostics render in on a multi-error fixture — the relocated rule now
# renders from a different stream — but it must never change the SET: the same
# (file, line, code, detail) records, one per emission. `test-accept.sh` diffs
# `build.log` byte-for-byte, so an expected reorder and an accidental wording or
# line drift look identical there. This harness tells them apart.
#
# For every golden `build.log`/`test.log` under tests/ that records at least one
# diagnostic, it re-runs the fixture's echoed `mfb build …`/`mfb test …` command
# lines exactly as echoed (the flags matter: a dump-only `-ast -ir` run stops
# before the link stage, whose `NATIVE_LIBRARY_*` warnings only a full build
# emits), removes whatever artifacts the run wrote into the fixture directory,
# and classifies the fixture:
#
#   SAME     identical records in identical order
#   REORDER  identical record multiset, different order (the expected shape of a
#            relocation's churn — list these fixtures in the relocation commit)
#   SETDIFF  the multiset differs (a rule went missing, doubled, or changed its
#            wording/line) — printed as a diff of the sorted records
#
# Exit status is non-zero iff any fixture is SETDIFF. A record is the diagnostic
# header line (`path:line severity[code NAME]: summary`) joined with the detail
# line that follows it, so wording drift in either is a SETDIFF.
#
# Usage: diag-set-diff.sh <mfb-exe> [-v] [name-glob ...]
#   -v         also print the before/after order for REORDER fixtures
#   name-glob  restrict to fixtures whose `tests/…` path matches (shell glob)
set -u
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO" || exit 2

if [ "$#" -lt 1 ]; then
  echo "Usage: $(basename "$0") <mfb-exe> [-v] [name-glob ...]" >&2
  exit 2
fi
MFB="$1"; shift
case "$MFB" in
  /*) ;;
  *) MFB="$REPO/$MFB" ;;
esac
if [ ! -x "$MFB" ]; then
  echo "not an executable: $MFB" >&2
  exit 2
fi
VERBOSE=0
if [ "${1:-}" = "-v" ]; then
  VERBOSE=1; shift
fi
GLOBS=("$@")

# The diagnostic header shape `show_diagnostic` renders (src/rules/mod.rs).
HEADER_RE='^[^ ].*:[0-9]+ (error|warn|info)\[[0-9-]+ [A-Z_]+\]: '

# Extract the diagnostic records from a log, in order: `header<TAB>detail` for
# a located diagnostic; an unlocated `error: RULE: detail` line (the merged-
# project gate's form) verbatim; and the `[exit N]` line of every `$ mfb …`
# section (not of a fixture's own executable run, which the harness does not
# replay), so a build that starts failing (or passing) is a SETDIFF even when
# its located diagnostics are unchanged.
records() {
  awk -v re="$HEADER_RE" '
    /^\$ mfb (build|test) / { in_mfb = 1; next }
    /^\$ / { in_mfb = 0; next }
    pending != "" { sub(/^[ \t]+/, ""); print pending "\t" $0; pending = ""; next }
    $0 ~ re { pending = $0; next }
    /^error: [A-Z_]+: / { print; next }
    /^\[exit [0-9]+\]$/ { if (in_mfb) print; next }
  ' "$1"
}

# Turn one echoed golden command line (`$ mfb build -ast -ir tests/x`) into the
# argv to run: everything after `mfb`, verbatim. `-q` is added at run time
# (test-accept passes it too; it only silences the `Building …` summary).
run_args() {
  local line=$1
  # shellcheck disable=SC2086
  set -- $line
  shift # `$`
  shift # `mfb`
  printf '%s\n' "$@"
}

# Delete every file the replayed build wrote into the fixture directory (the
# artifact dumps `-ast`/`-ir`/… produce, a linked executable, the `build/`
# output dir) so the working tree is left exactly as found.
snapshot_files() {
  find "$1" -type f | sort
}
remove_new_files() {
  local dir=$1 before=$2
  snapshot_files "$dir" | comm -13 "$before" - | while IFS= read -r f; do rm -f "$f"; done
  [ -d "$dir/build" ] && [ "$3" -eq 0 ] && rm -rf "$dir/build"
  return 0
}

matches_glob() {
  local name=$1 g
  [ "${#GLOBS[@]}" -eq 0 ] && return 0
  for g in "${GLOBS[@]}"; do
    # shellcheck disable=SC2254
    case "$name" in $g) return 0 ;; esac
  done
  return 1
}

same=0; reorder=0; setdiff=0; ran=0
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

while IFS= read -r golden; do
  fixture_dir="$(dirname "$(dirname "$golden")")"
  name="${fixture_dir#"$REPO/"}"
  matches_glob "$name" || continue
  records "$golden" > "$tmp/expected"
  [ -s "$tmp/expected" ] || continue
  ran=$((ran + 1))
  had_build_dir=0
  [ -d "$fixture_dir/build" ] && had_build_dir=1
  snapshot_files "$fixture_dir" > "$tmp/before"
  : > "$tmp/actual"
  while IFS= read -r cmd; do
    # bash 3.2 (macOS) has no `mapfile`; collect the argv line by line.
    argv=()
    while IFS= read -r tok; do argv+=("$tok"); done < <(run_args "$cmd")
    verb="${argv[0]}"; unset 'argv[0]'
    # Diagnostics are on stderr; the harness merges both streams and records
    # the exit status exactly as test-accept does.
    {
      echo "$cmd"
      "$MFB" "$verb" -q "${argv[@]}" 2>&1 < /dev/null
      echo "[exit $?]"
    } > "$tmp/run.out"
    records "$tmp/run.out" >> "$tmp/actual"
  done < <(grep -E '^\$ mfb (build|test) ' "$golden")
  remove_new_files "$fixture_dir" "$tmp/before" "$had_build_dir"
  if cmp -s "$tmp/expected" "$tmp/actual"; then
    same=$((same + 1))
    echo "SAME     $name"
    continue
  fi
  sort "$tmp/expected" > "$tmp/expected.sorted"
  sort "$tmp/actual" > "$tmp/actual.sorted"
  if cmp -s "$tmp/expected.sorted" "$tmp/actual.sorted"; then
    reorder=$((reorder + 1))
    echo "REORDER  $name"
    if [ "$VERBOSE" -eq 1 ]; then
      diff -u --label "$name (golden order)" --label "$name (actual order)" \
        "$tmp/expected" "$tmp/actual" | sed 's/^/    /'
    fi
    continue
  fi
  setdiff=$((setdiff + 1))
  echo "SETDIFF  $name"
  diff -u --label "$name (golden set)" --label "$name (actual set)" \
    "$tmp/expected.sorted" "$tmp/actual.sorted" | sed 's/^/    /'
done < <(grep -rlE --include=build.log --include=test.log ' (error|warn|info)\[' "$REPO/tests" | grep '/golden/' | sort)

echo "diag-set-diff: $ran fixture(s) with diagnostics — $same same, $reorder reordered, $setdiff set-diff"
[ "$setdiff" -eq 0 ]

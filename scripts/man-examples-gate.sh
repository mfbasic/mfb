#!/usr/bin/env bash
#
# man-examples-gate.sh — the documentation-example gate (bug-472).
#
# Every Examples block on every built-in package's rendered `mfb man` pages must
# BUILD, and must RUN, unless it is listed in the not-run file with its reason.
# It is a check, not a fixer: nothing here rewrites an example.
#
#   man-examples-gate.sh [path/to/mfb]
#
# The per-package work is `scripts/man-run-examples.sh`, which lifts each block
# out of RENDERED man output — what is checked is what a reader would type.
#
# `testing` runs through `--test`: `mfb build` DROPS TESTING blocks before
# codegen, so building and running a `testing` example proves nothing about the
# block it exists to show.
#
# The not-run file (`scripts/man-examples-not-run.txt`, override with
# NOT_RUN_FILE) must not rot: an entry whose example no longer exists is a
# failure, so deleting or renumbering an example forces its entry to go too.
#
# Env: RUN_TIMEOUT (seconds per example, default 60), WORK (scratch root,
#      default a fresh mktemp dir), NOT_RUN_FILE.

set -uo pipefail

MFB=${1:-./target/release/mfb}
root=$(cd "$(dirname "$0")/.." && pwd)
NOT_RUN_FILE=${NOT_RUN_FILE:-$root/scripts/man-examples-not-run.txt}
# Every example gets this on stdin, so the `io` pages that read a line, a
# character or a byte are verified by RUNNING rather than excused. A CI step's
# stdin is not a terminal and may be empty, which would otherwise be EOF.
STDIN_FILE=${STDIN_FILE:-$root/scripts/man-examples-stdin.txt}
export STDIN_FILE
WORK=${WORK:-$(mktemp -d "${TMPDIR:-/tmp}/man-examples-gate.XXXXXX")}

if [ ! -x "$MFB" ]; then
	echo "man-examples-gate.sh: no mfb binary at $MFB" >&2
	exit 2
fi
if [ ! -f "$NOT_RUN_FILE" ]; then
	echo "man-examples-gate.sh: no not-run file at $NOT_RUN_FILE" >&2
	exit 2
fi
mkdir -p "$WORK" || exit 2
MFB=$(cd "$(dirname "$MFB")" && pwd)/$(basename "$MFB")
export MFB NOT_RUN_FILE

# The package column of `mfb man`'s index table. Continuation rows have an empty
# first cell and the header is capitalised, so only package names match. The
# index goes on to a "Guide topics" table of the same shape (`errors`, `flow`,
# …); those are narrative pages, not packages, so stop reading at its heading.
packages=$("$MFB" man 2>/dev/null | awk -F'│' '/^Guide topics$/ { exit } NF >= 3 {
	name = $2; gsub(/ /, "", name)
	if (name ~ /^[a-z][A-Za-z0-9]*$/) print name
}')
if [ -z "$packages" ]; then
	echo "man-examples-gate.sh: \`$MFB man\` listed no packages" >&2
	exit 2
fi
# `general` (the bare-name global builtins) renders function pages but is kept
# out of the index because it has no `IMPORT` spelling. Its pages carry examples
# all the same, so name it here — and fail if it stops rendering, rather than let
# the gate silently shrink.
for unindexed in general; do
	if ! "$MFB" man "$unindexed" >/dev/null 2>&1; then
		echo "man-examples-gate.sh: \`$MFB man $unindexed\` no longer renders" >&2
		exit 2
	fi
	printf '%s\n' "$packages" | grep -qx "$unindexed" || packages="$packages
$unindexed"
done

grand_total=0
grand_failed=0
stale=0
failures=""
gate_start=$SECONDS

printf '%-12s %8s %6s %6s %8s %7s %6s\n' package examples built ran "not run" failed secs
for pkg in $packages; do
	mode=--run
	[ "$pkg" = testing ] && mode=--test
	log=$WORK/$pkg.log
	start=$SECONDS
	SCRATCH=$WORK/$pkg bash "$root/scripts/man-run-examples.sh" "$pkg" "$mode" >"$log" 2>&1
	summary=$(grep -E '^examples: ' "$log" | tail -1)
	if [ -z "$summary" ]; then
		echo "$pkg: the runner printed no summary — see $log" >&2
		grand_failed=$((grand_failed + 1))
		failures="$failures $pkg(no-summary)"
		continue
	fi
	field() { printf '%s\n' "$summary" | sed -nE "s/.*$1: ([0-9]+).*/\\1/p"; }
	total=$(field examples)
	built=$(field built)
	ran=$(field ran)
	not_run=$(field "not run")
	failed=$(field failed)
	printf '%-12s %8s %6s %6s %8s %7s %6s\n' "$pkg" "$total" "$built" "$ran" "${not_run:-0}" "$failed" $((SECONDS - start))
	grand_total=$((grand_total + total))
	grand_failed=$((grand_failed + failed))
	if [ "$failed" -ne 0 ]; then
		failures="$failures $(grep -E '^failures:' "$log" | sed 's/^failures://')"
	fi

	# Every not-run entry for this package must name an example the runner saw.
	while read -r id _; do
		case $id in "$pkg::"*) ;; *) continue ;; esac
		fn=${id#"$pkg::"}
		n=${fn##*#}
		fn=${fn%#*}
		if ! grep -qF "=== $pkg::$fn example $n — compiled; not run:" "$log"; then
			echo "stale not-run entry: $id (no such example, or it no longer builds)" >&2
			stale=$((stale + 1))
		fi
	done < <(grep -vE '^[[:space:]]*(#|$)' "$NOT_RUN_FILE")
done

echo
echo "man examples: $grand_total checked, $grand_failed failed, $stale stale not-run entr(y/ies), $((SECONDS - gate_start))s"
[ -n "$failures" ] && echo "failures:$failures"
echo "logs: $WORK"

if [ "$grand_total" -eq 0 ]; then
	echo "man-examples-gate.sh: no examples were checked — the extractor is broken" >&2
	exit 1
fi
[ "$grand_failed" -eq 0 ] && [ "$stale" -eq 0 ]

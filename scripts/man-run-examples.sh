#!/usr/bin/env bash
#
# man-run-examples.sh — compile (and optionally run) every Examples code block
# on a package's rendered man pages.
#
# plan-108 requires that every example on a page was actually compiled and run
# while the page was written; A's census measured ZERO prior example
# verification across the whole surface. This is the instrument for that.
#
#   man-run-examples.sh <pkg> [--run|--test] [fn...]
#
# Without --run each block is only compiled (use for tty / device / live-endpoint
# members). With --run a successful build is executed and its stdout shown, so
# the author can compare it against what the page claims.
#
# --test runs `mfb test` instead of build-then-execute. Use it for examples whose
# point is a TESTING block: an ordinary `mfb build` DROPS those blocks before
# codegen, so a clean build proves nothing about them.
#
# Blocks are lifted from RENDERED output, so what is checked is exactly what a
# developer reading the page would type. A block starts at an `IMPORT` line and
# runs to the end of the indented region.
#
# Env: MFB (default ./target/release/mfb), SCRATCH (default /tmp/man-examples)

set -uo pipefail

MFB=${MFB:-./target/release/mfb}
SCRATCH=${SCRATCH:-/tmp/man-examples}
export LC_ALL=${LC_ALL:-en_US.UTF-8}

pkg=${1:?usage: man-run-examples.sh <pkg> [--run] [fn...]}
shift
run=0
test_mode=0
case "${1:-}" in
--run)
	run=1
	shift
	;;
--test)
	test_mode=1
	shift
	;;
esac

if [ ! -x "$MFB" ]; then
	echo "man-run-examples.sh: no mfb binary at $MFB" >&2
	exit 2
fi

functions() {
	if [ "$#" -gt 0 ]; then
		printf '%s\n' "$@"
	else
		"$MFB" man "$pkg" 2>/dev/null |
			grep -oE "^│ ${pkg}::[A-Za-z0-9_]+" |
			sed "s/^│ ${pkg}:://" | sort -u
	fi
}

# The scratch project is rebuilt per block; SCRATCH is ours alone and is only
# ever a path we just created (never a caller-supplied directory).
#
# PROJECT overrides it with an already-prepared project, for packages whose
# examples need dependencies a bare `mfb init` cannot supply — `thread`'s
# examples all call into a companion worker package, because a thread entry
# point MUST be an exported ISOLATED FUNC reached through an import. Only
# src/main.mfb is replaced; the project's manifest and packages/ are left alone.
prepare_project() {
	if [ -n "${PROJECT:-}" ]; then
		SCRATCH=$PROJECT
		[ -d "$SCRATCH/src" ] || return 1
		return 0
	fi
	rm -rf "$SCRATCH"
	"$MFB" init "$SCRATCH" >/dev/null 2>&1 || return 1
}

total=0
built=0
ran=0
failed=0
failed_list=""

for fn in $(functions "$@"); do
	page=$("$MFB" man "$pkg" "$fn" 2>/dev/null)

	# Split the Examples section into blocks. A block starts at an `IMPORT`
	# line and continues while lines stay indented (blank lines included); the
	# first non-indented, non-blank line is the prose introducing the NEXT
	# block, and ends this one.
	blocks=$(printf '%s\n' "$page" | awk '
		/^Examples$/ { inex = 1; next }
		!inex { next }
		# A bare capitalised word on its own line is the next section heading.
		/^[A-Za-z][A-Za-z ]*$/ { inex = 0; inblock = 0; next }
		{
			# Only the FIRST IMPORT opens a block — examples routinely start
			# with several (`IMPORT bits` then `IMPORT io`).
			if ($0 ~ /^  IMPORT / && !inblock) { inblock = 1; n++; print "###BLOCK" n }
			else if (inblock && $0 !~ /^  / && $0 ~ /[^ ]/) { inblock = 0 }
			if (inblock) { line = $0; sub(/^  /, "", line); print line }
		}
	')
	[ -z "$blocks" ] && continue

	count=$(printf '%s\n' "$blocks" | grep -c "^###BLOCK")
	i=0
	while [ "$i" -lt "$count" ]; do
		i=$((i + 1))
		total=$((total + 1))
		src=$(printf '%s\n' "$blocks" |
			awk -v want="$i" '
				/^###BLOCK/ { cur = substr($0, 9) + 0; next }
				cur == want { print }
			')

		prepare_project || { echo "SETUP-FAIL $pkg::$fn #$i"; continue; }
		printf '%s\n' "$src" > "$SCRATCH/src/main.mfb"

		if [ "$test_mode" = 1 ]; then
			# `mfb test` exits non-zero iff a case failed, which is exactly the
			# signal we want: the example's own assertions are the check.
			if result=$("$MFB" test "$SCRATCH" 2>&1); then
				built=$((built + 1))
				ran=$((ran + 1))
				echo "=== $pkg::$fn example $i — mfb test passed ==="
				printf '%s\n' "$result" | tail -20
			else
				failed=$((failed + 1))
				failed_list="$failed_list $pkg::$fn#$i(test)"
				echo "=== $pkg::$fn example $i — mfb test FAILED ==="
				printf '%s\n' "$result" | tail -20
			fi
			continue
		fi

		if out=$("$MFB" build "$SCRATCH" 2>&1); then
			built=$((built + 1))
			if [ "$run" = 1 ]; then
				bin=$(find "$SCRATCH/build" -name '*.out' -type f 2>/dev/null | head -1)
				if [ -n "$bin" ] && result=$("$bin" 2>&1); then
					ran=$((ran + 1))
					echo "=== $pkg::$fn example $i — ran ==="
					printf '%s\n' "$result"
				else
					failed=$((failed + 1))
					failed_list="$failed_list $pkg::$fn#$i(run)"
					echo "=== $pkg::$fn example $i — RUN FAILED ==="
					printf '%s\n' "${result:-<no output>}"
				fi
			else
				echo "=== $pkg::$fn example $i — compiled ==="
			fi
		else
			failed=$((failed + 1))
			failed_list="$failed_list $pkg::$fn#$i(build)"
			echo "=== $pkg::$fn example $i — BUILD FAILED ==="
			printf '%s\n' "$out" | tail -12
		fi
	done
done

echo
echo "examples: $total   built: $built   ran: $ran   failed: $failed"
[ -n "$failed_list" ] && echo "failures:$failed_list"
[ "$failed" -eq 0 ]

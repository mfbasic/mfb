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
# Env: MFB (default ./target/release/mfb), SCRATCH (default /tmp/man-examples),
#      STDIN_FILE (a file piped to each example's stdin; unset = inherit)

set -uo pipefail

MFB=${MFB:-./target/release/mfb}
SCRATCH=${SCRATCH:-/tmp/man-examples}
STDIN_FILE=${STDIN_FILE:-}
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
	install_workers_package "$SCRATCH"
}

# `thread` (and `os::sleep`'s worker example) call into a companion package
# because a thread entry point must be an exported ISOLATED FUNC reached
# through an import. Rather than require a hand-prepared PROJECT, build the
# companion here so those pages are verified by RUNNING, not compiling. The
# signatures are dictated by the examples themselves — change one and the
# example stops building, which is the point. The compiled .mfp is built once
# per invocation and copied into each scratch project.
WORKERS_SRC=${SCRATCH}-workers-src
WORKERS_MFP=$WORKERS_SRC/workers.mfp

build_workers_package() {
	rm -rf "$WORKERS_SRC"
	mkdir -p "$WORKERS_SRC/src"
	cat > "$WORKERS_SRC/project.json" <<-'JSON'
	{
	  "name": "workers",
	  "version": "0.1.0",
	  "mfb": "1.0",
	  "kind": "package",
	  "description": "Companion worker package for the thread and os man-page examples.",
	  "sources": [{ "root": "src", "role": "lib", "include": ["**/*.mfb"] }]
	}
	JSON
	cat > "$WORKERS_SRC/src/workers.mfb" <<-'MFB'
	IMPORT thread
	IMPORT os
	IMPORT fs
	IMPORT strings

	EXPORT ISOLATED FUNC double(w AS ThreadWorker OF Nothing TO Integer, seed AS Integer) AS Integer
	  RETURN seed * 2
	END FUNC

	EXPORT ISOLATED FUNC chatter(worker AS ThreadWorker OF String TO Integer, greeting AS String) AS Integer
	  thread::send(worker, greeting & " from the worker")
	  LET reply AS String = thread::receive(worker, 1000)
	  thread::send(worker, "worker heard: " & reply)
	  RETURN len(reply)
	END FUNC

	EXPORT ISOLATED FUNC patient(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
	  MUT spins AS Integer = 0
	  WHILE NOT thread::isCancelled(w)
	    os::sleep(10)
	    spins = spins + 1
	    IF spins > 500 THEN
	      RETURN spins
	    END IF
	  END WHILE
	  RETURN spins
	END FUNC

	EXPORT ISOLATED FUNC failing(w AS ThreadWorker OF Nothing TO Integer, seed AS Integer) AS Integer
	  ' strings::mid raises when the count runs past the end of the string.
	  RETURN len(strings::mid("a", 0, seed + 10))
	END FUNC

	EXPORT ISOLATED FUNC fileWriter(w AS ThreadWorker OF RES fs::File TO Integer, seed AS Integer) AS Integer
	  RES f AS fs::File = thread::accept(w)
	  fs::writeAll(f, "from the worker\n")
	  fs::close(f)
	  RETURN 0
	END FUNC

	EXPORT ISOLATED FUNC tick(w AS ThreadWorker OF Nothing TO String, seed AS Integer) AS String
	  os::sleep(5000) TRAP(err)
	    RETURN "cancelled"
	  END TRAP
	  RETURN "finished"
	END FUNC
	MFB
	"$MFB" build "$WORKERS_SRC" >/dev/null 2>&1 || {
		echo "WORKERS-BUILD-FAILED — rerun: $MFB build $WORKERS_SRC" >&2
		return 1
	}
	[ -f "$WORKERS_MFP" ]
}

install_workers_package() {
	root=$1
	[ -f "$WORKERS_MFP" ] || return 0
	mkdir -p "$root/packages"
	cp "$WORKERS_MFP" "$root/packages/workers.mfp"
	python3 - "$root/project.json" <<-'PY'
	import json, sys
	p = sys.argv[1]
	d = json.load(open(p))
	pkgs = [x for x in d.get("packages", []) if x.get("name") != "workers"]
	pkgs.append({"name": "workers", "version": "=0.1.0",
	             "source": "file:packages/workers.mfp"})
	d["packages"] = pkgs
	json.dump(d, open(p, "w"), indent=2)
	PY
}

build_workers_package || true

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
				# STDIN_FILE feeds real input to examples that read stdin, so an
				# io/term page is verified by running rather than written off as
				# compile-only.
				# Run with the scratch PROJECT as cwd. Running from the
				# repository root instead makes a relative path like
				# "target/output.txt" resolve against cargo's own target/,
				# so an example that would fail for a reader passes here.
				if [ -z "$bin" ]; then
					rc=1
				elif [ -n "$STDIN_FILE" ]; then
					result=$(cd "$SCRATCH" && "$bin" <"$STDIN_FILE" 2>&1) && rc=0 || rc=1
				else
					result=$(cd "$SCRATCH" && "$bin" 2>&1) && rc=0 || rc=1
				fi
				# An example may document its own failure — `groupBy`'s third
				# shows a propagating error and the page says "prints, and exits
				# non-zero: failed: 77050002". A non-zero exit is only a real
				# failure when the page does NOT show what the program printed,
				# so this checks the output against the rendered page instead of
				# trusting the exit status alone.
				documented=0
				if [ -n "$bin" ] && [ "$rc" != 0 ] && [ -n "$result" ]; then
					documented=1
					while IFS= read -r out_line; do
						[ -z "$out_line" ] && continue
						case $page in
						*"$out_line"*) ;;
						*) documented=0 ;;
						esac
					done <<-EOF
					$result
					EOF
				fi
				if [ -n "$bin" ] && { [ "$rc" = 0 ] || [ "$documented" = 1 ]; }; then
					ran=$((ran + 1))
					if [ "$documented" = 1 ]; then
						echo "=== $pkg::$fn example $i — ran (documented non-zero exit) ==="
					else
						echo "=== $pkg::$fn example $i — ran ==="
					fi
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

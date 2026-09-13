#!/usr/bin/env bash
#
# man-run-examples.sh — compile (and optionally run) every Examples code block
# on a package's rendered man pages.
#
# plan-108 requires that every example on a page was actually compiled and run
# while the page was written; A's census measured ZERO prior example
# verification across the whole surface. This is the instrument for that.
#
#   man-run-examples.sh <pkg>   [--run|--test] [fn...]
#   man-run-examples.sh <topic> --topic [--run]
#
# --topic treats the argument as a narrative guide topic (`variable`, `tour`, …)
# rather than a registry package. A topic has no Functions table and no
# `Examples` section — its code blocks sit inline throughout the prose — so
# without this the script reports "examples: 0" for it, which reads as "nothing
# to check" rather than "this instrument cannot see 135 code blocks".
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
# A TOPIC is the one exception, and for a reason, not for convenience: a topic
# alternates a ```basic program with a bare ``` block holding the output it
# prints, and rendered, both are just indented two spaces with a blank line
# between. Nothing in the rendering tells them apart, so a rendered-text
# extractor glues each program to its own expected output and every block fails
# to compile (measured: 10 of 10 on `variable`). The language tag is the
# discriminator and it exists only in the source, so topic mode reads the
# markdown under src/docs/man/<topic>/.
#
# Env: MFB (default ./target/release/mfb), SCRATCH (default /tmp/man-examples),
#      STDIN_FILE (a file piped to each example's stdin; unset = inherit),
#      RUN_TIMEOUT (seconds one example may run; default 60)

set -uo pipefail

MFB=${MFB:-./target/release/mfb}
SCRATCH=${SCRATCH:-/tmp/man-examples}
STDIN_FILE=${STDIN_FILE:-}
# Seconds any one example may run before it is killed and reported as a failure.
RUN_TIMEOUT=${RUN_TIMEOUT:-60}

# `timeout`/`gtimeout` are not on a stock macOS, so bound the run by hand: start
# the example, start a killer, and take whichever finishes first. Exit 124 marks
# a timeout, matching coreutils `timeout`.
#
# The killer's stdout MUST go to /dev/null. This whole function runs inside a
# `$(...)`, and a command substitution does not return until every process
# holding the pipe's write end has exited — so a killer that inherits stdout
# makes each example take the full timeout even when it finished instantly.
# Killing the subshell is likewise not enough: `sleep` is its own process and
# keeps the pipe, so kill the process GROUP.
run_bounded() {
	# A background job in a non-interactive shell gets /dev/null on stdin unless
	# it is redirected explicitly, so hand it this function's own stdin — which
	# is where STDIN_FILE arrives. Without this, every example that reads input
	# sees EOF and the STDIN_FILE plumbing above is silently inert.
	exec 3<&0
	"$@" <&3 &
	run_pid=$!
	{ sleep "$RUN_TIMEOUT"; kill -9 "$run_pid" 2>/dev/null; } >/dev/null 2>&1 &
	killer_pid=$!
	wait "$run_pid" 2>/dev/null
	run_rc=$?
	kill -- "-$killer_pid" 2>/dev/null || kill "$killer_pid" 2>/dev/null
	wait "$killer_pid" 2>/dev/null
	# kill -9 surfaces as 137 (128 + SIGKILL).
	[ "$run_rc" = 137 ] && return 124
	return "$run_rc"
}
export LC_ALL=${LC_ALL:-en_US.UTF-8}

pkg=${1:?usage: man-run-examples.sh <pkg|topic> [--topic] [--run|--test] [fn...]}
shift
run=0
test_mode=0
# A narrative guide topic is not a registry package: it has no Functions table to
# scrape and no `Examples` section — its code blocks sit inline throughout the
# prose. `.ai/man-content.md` §9 holds a topic's blocks to the same
# compile-and-run rule as a function page's example, and 135 of them exist
# (`man-census.sh --topics`), so they need an instrument too.
topic_mode=0
case "${1:-}" in
--topic)
	topic_mode=1
	shift
	;;
esac
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
	# A topic is one unit: its overview and every subtopic are swept together,
	# so there is a single pseudo-page.
	if [ "$topic_mode" = 1 ]; then
		printf '%s\n' 'guide'
		return
	fi
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
	[ "$needs_workers" = 1 ] && install_workers_package "$SCRATCH"
	[ "$needs_canvas_fixtures" = 1 ] && install_canvas_fixtures "$SCRATCH"
	return 0
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

# The data files the `canvas` pages' examples open.
#
# Same reasoning as the companion package above: an example that loads a font or an
# image is only *verified* if it can actually run, and without these five canvas pages
# were reported as run-failures on every invocation
# (`canvas::loadFont`, `loadImage`, `fontRef`, `destroyFont`, `measureText` — all
# "Filesystem path does not exist"). The alternative was to rewrite the examples not to
# open a file, which would document something other than what the function is for.
#
# The names are dictated by the examples themselves, exactly as the worker signatures
# are: rename one in a page and it stops running here, which is the point.
#
# The font is the same synthesized twelve-glyph TrueType `tests/rt_canvas_font.rs` and
# `scripts/test-canvas-vulkan.sh` build — `unitsPerEm` 1000, one square glyph — rather
# than a real typeface, so the harness needs nothing installed on the machine.
install_canvas_fixtures() {
	root=$1
	base64 -d > "$root/DejaVuSans.ttf" <<-'TTF'
	AAEAAAAGAAAAAAAAY21hcAAAAAAAAABsAAAANGdseWYAAAAAAAAAoAAAACJoZWFkAAAAAAAAAMIA
	AAA2aGhlYQAAAAAAAAD4AAAAJGhtdHgAAAAAAAABHAAAAAxsb2NhAAAAAAAAASgAAAAIAAAAAQAD
	AAoAAAAMAAwAAAAAACgAAAAAAAAAAgAAAEEAAABBAAAAAQAAAEIAAABCAAAAAgABAGQAAAGQASwA
	AwAAAQEBAQBkASwAAP7UAAAAAAEsAAAAAAAAAAAAAAAAAAAAAAAAAAAD6AAAAAAAAAAAAAAAAAAA
	AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAyD/OABkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMB
	9AAAAPoAAAEsAAAAAAAAABEAEQ==
	TTF
	# A 2x2 RGBA PNG. Small on purpose: the examples only need `loadImage` to succeed
	# and report a size, and a large asset would make this script carry a payload.
	base64 -d > "$root/logo.png" <<-'PNG'
	iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEklEQVR42mP4z8DwHwyBNBgAAEnI
	CfcD2WTxAAAAAElFTkSuQmCC
	PNG
}

# The companion package costs a full package build, so only pay for it when
# this package's pages actually import it.
needs_canvas_fixtures=0
if "$MFB" man "$pkg" --all 2>/dev/null | grep -qE '"(DejaVuSans\.ttf|logo\.png)"'; then
	needs_canvas_fixtures=1
fi

needs_workers=0
if "$MFB" man "$pkg" --all 2>/dev/null | grep -q '^  IMPORT workers'; then
	needs_workers=1
	build_workers_package || true
fi

# One extracted code block: stage it, build it, optionally run it, and update
# the counters. Called from both paths — the registry-package path, which lifts
# blocks from a rendered `Examples` section, and the guide-topic path, which
# lifts ```basic fences from the topic's markdown. Factored out so those two
# extractors cannot drift in how they JUDGE a block, only in how they find one.
#
# Deliberately not run in a subshell: it updates `total`/`built`/`ran`/`failed`
# and `failed_list` in the caller's shell, and a `$( )` would silently discard
# every count.
#
# $1 = source text, $2 = index within this page/topic
run_one_block() {
	local src=$1 i=$2

	prepare_project || { echo "SETUP-FAIL $pkg::$fn #$i"; return 0; }
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
		return 0
	fi

	build_flags=""
	case $pkg in app|canvas) build_flags="--app" ;; esac
	if out=$("$MFB" build "$SCRATCH" $build_flags 2>&1); then
		built=$((built + 1))
		# NOT_RUN_FILE lists examples that cannot run on this host, one
		# `pkg::fn#N  reason` per line. A listed example must still BUILD;
		# only the run is skipped, and it is reported with its reason so a
		# skip is never silent.
		not_run_reason=""
		if [ "$run" = 1 ] && [ -n "${NOT_RUN_FILE:-}" ]; then
			not_run_reason=$(awk -v id="$pkg::$fn#$i" \
				'$1 == id { $1 = ""; sub(/^ +/, ""); print; exit }' "$NOT_RUN_FILE")
		fi
		# A reason starting `serves:` marks a server example whose accept loop
		# never returns by design. It is not skipped: it runs for SERVE_SECONDS
		# and passes only if it is STILL RUNNING when killed, so a server that
		# fails to bind or dies on start is still caught.
		serves=0
		case $not_run_reason in serves:*) serves=1 ;; esac
		if [ -n "$not_run_reason" ] && [ "$serves" = 0 ]; then
			not_run=$((not_run + 1))
			echo "=== $pkg::$fn example $i — compiled; not run: $not_run_reason ==="
		elif [ "$serves" = 1 ]; then
			bin=$(find "$SCRATCH/build" -name '*-glibc.out' -type f 2>/dev/null | head -1)
			[ -z "$bin" ] && bin=$(find "$SCRATCH/build" -name '*.out' -type f 2>/dev/null | head -1)
			rc=1
			result="<no console binary>"
			if [ -n "$bin" ]; then
				result=$(cd "$SCRATCH" && RUN_TIMEOUT=${SERVE_SECONDS:-3} run_bounded "$bin" </dev/null 2>&1) && rc=0 || rc=$?
			fi
			if [ "$rc" = 124 ]; then
				ran=$((ran + 1))
				echo "=== $pkg::$fn example $i — serves (still running after ${SERVE_SECONDS:-3}s): ${not_run_reason#serves:} ==="
			else
				failed=$((failed + 1))
				failed_list="$failed_list $pkg::$fn#$i(serve)"
				echo "=== $pkg::$fn example $i — SERVE FAILED (exited $rc before ${SERVE_SECONDS:-3}s) ==="
				printf '%s\n' "${result:-<no output>}"
			fi
		elif [ "$run" = 1 ]; then
			# A Linux console build emits BOTH `<name>-glibc.out` and
			# `<name>-musl.out`; run the glibc one so every Linux run is
			# the same libc world rather than whichever `find` lists first.
			run_args=""
			bin=$(find "$SCRATCH/build" -name '*-glibc.out' -type f 2>/dev/null | head -1)
			[ -z "$bin" ] && bin=$(find "$SCRATCH/build" -name '*.out' -type f 2>/dev/null | head -1)
			if [ -z "$bin" ]; then
				# `-perm -u+x`, not `-perm +111`: GNU find rejects the `+`
				# form outright ("invalid file mode"), BSD find takes both.
				bin=$(find "$SCRATCH/build" -path '*.app/Contents/MacOS/*' \
					-type f -perm -u+x 2>/dev/null | head -1)
				# A Linux `--app` build seals `<name>-glibc.AppImage` (plus a
				# musl one). A CI runner has no FUSE, so extract-and-run it.
				if [ -z "$bin" ]; then
					bin=$(find "$SCRATCH/build" -name '*-glibc.AppImage' -type f 2>/dev/null | head -1)
					[ -n "$bin" ] && run_args="--appimage-extract-and-run"
				fi
				# An app-mode program has no stdout: io::print goes to the
				# application transcript. Running still proves it starts.
				export MFB_MACAPP_HEADLESS=1 MFB_GTKAPP_HEADLESS=1
			fi
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
				result=$(cd "$SCRATCH" && run_bounded "$bin" ${run_args:+"$run_args"} <"$STDIN_FILE" 2>&1) && rc=0 || rc=$?
			else
				result=$(cd "$SCRATCH" && run_bounded "$bin" ${run_args:+"$run_args"} 2>&1) && rc=0 || rc=$?
			fi
			if [ "$rc" = 124 ]; then
				result="TIMED OUT after ${RUN_TIMEOUT}s — an example must terminate.
$result"
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
}

total=0
built=0
ran=0
failed=0
failed_list=""
not_run=0

for fn in $(functions "$@"); do
	# A guide topic is read from its MARKDOWN, not from the rendering, and this
	# is the one place that is the honest choice rather than a shortcut.
	#
	# A topic alternates a ```basic program with a bare ``` block holding the
	# output it prints. Rendered, both are simply indented by two spaces with a
	# blank line between, and NOTHING in the rendering distinguishes them — an
	# extractor working from rendered text glues each program to its own
	# expected output and every block fails to compile with the output text
	# parsed as source. (Measured: 10 of 10 on `variable`.) A function page has
	# no such pairs, which is why the registry path can and does read rendered
	# output. Here the language tag is the discriminator, and it only exists in
	# the source.
	if [ "$topic_mode" = 1 ]; then
		# `page` feeds the documented-non-zero-exit check further down: an
		# example is allowed to fail if the page shows what it printed. The
		# rendered topic is that reference text.
		page=$("$MFB" man "$pkg" --all 2>/dev/null)
		blocks=$(find "${MANDOCS:-src/docs/man}/$pkg" -name '*.md' | sort | while IFS= read -r f; do
			awk '
				/^```basic$/ { inb = 1; n++; print "###BLOCK" n; next }
				/^```/       { inb = 0; next }
				inb          { print }
			' "$f"
		done | awk '
			# Renumber across files so the ###BLOCK indices stay unique and
			# monotonic; each file restarts its own count at 1.
			/^###BLOCK/ { n++; print "###BLOCK" n; next }
			{ print }
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
			run_one_block "$src" "$i" || true
		done
		continue
	fi

	page=$("$MFB" man "$pkg" "$fn" 2>/dev/null)

	# Split the Examples section into blocks. A block starts at an `IMPORT`
	# line and continues while lines stay indented (blank lines included); the
	# first non-indented, non-blank line is the prose introducing the NEXT
	# block, and ends this one.
	blocks=$(printf '%s\n' "$page" | awk -v anywhere="$topic_mode" '
		# A guide topic has no `Examples` section — its blocks are inline in the
		# prose — so topic mode opens the gate at line 1 and never closes it on a
		# heading. Nothing is lost: a heading is non-indented and non-blank, so
		# the `inblock` rule below already ends the block it follows.
		BEGIN { if (anywhere) inex = 1 }
		/^Examples$/ { inex = 1; next }
		!inex { next }
		# A bare capitalised word on its own line is the next section heading.
		!anywhere && /^[A-Za-z][A-Za-z ]*$/ { inex = 0; inblock = 0; next }
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
		run_one_block "$src" "$i" || true
	done
done

echo
echo "examples: $total   built: $built   ran: $ran   not run: $not_run   failed: $failed"
[ -n "$failed_list" ] && echo "failures:$failed_list"
[ "$failed" -eq 0 ]

#!/usr/bin/env bash
#
# doc-review-fanout.sh — plan-125's parallel second-reviewer harness.
#
# Runs one headless Claude Code review (`claude -p`, model Claude Fable 5.1 by
# default) per review unit across N reusable detached worktrees, and — this is
# the part that matters — records an outcome for EVERY unit.
#
# WHY FABLE, NOT CODEX (owner decision, 2026-09-25)
#
#   Letters A through D Phase 3 (partial) ran `codex exec`. The Codex plan's usage
#   limit stalled D Phase 3 twice, so the owner switched the reviewer to Claude
#   Fable 5.1. This reverses plan-125-A's "use another Claude tier" rejection:
#   the reviewer is now a different model from the main thread, not a different
#   vendor. Override with REVIEWER_MODEL=<model id> if a letter needs another.
#
#   Reviewer worktrees live under /tmp on purpose, and must stay there: a
#   `claude` session whose cwd is inside `.claude/worktrees/P-*` trips the
#   follow-plan Stop hook, which will not let it stop while the plan has open
#   boxes — the run never returns (measured 2026-09-25 with the PONG probe).
#
# WHY THE MANIFEST IS THE POINT
#
#   plan-125 makes ~900 reviewer runs. Its single most likely failure is not a
#   bad review: it is a unit that silently never ran and is counted as clean.
#   A harness that drops a failed run reports a cleaner surface than it
#   measured, and nothing downstream can tell. So:
#
#     * every unit gets a manifest row, including the ones that fail;
#     * a run that exits non-zero, times out, or produces an EMPTY findings
#       file is re-queued exactly once and then recorded FAILED — never
#       dropped, never silently retried forever;
#     * --reconcile exits non-zero if any listed unit is missing from the
#       manifest, FAILED, or has no findings file.
#
#   (memory: diagnostic-harness-must-record-exit-and-unlocated-errors — a
#   failed run must never read as "same".)
#
# WHY THE REVIEWER NEVER COMMITS
#
#   The reviewer runs with permissions bypassed (headless: nobody is there to
#   approve a prompt) so it can build probe programs and run `mfb`, and its
#   `git commit` / `git push` are denied outright. Unlike Codex's
#   `workspace-write`, that is not an OS sandbox: a reviewer CAN write anywhere,
#   including network probes Codex could not make. The main thread in the
#   primary checkout is still the only writer of the repository.
#   After every unit the harness runs `git status --porcelain` in that unit's
#   worktree; a non-empty result is recorded DIRTY in the manifest and the
#   worktree is reset before it is reused.
#   (memory: subagent-edits-can-silently-vanish.)
#
# WHY THE WORKTREES CARRY NO target/
#
#   Six concurrent cargo builds would dwarf the reviews. The worktrees are
#   fresh `git worktree add` checkouts, so `target/` (gitignored) does not
#   exist in them; reviewers are handed MFB=<primary>/target/release/mfb and
#   render pages with the already-built binary.
#
# SCRATCH
#
#   Probe programs go in /tmp/plan-125-scratch/<letter>/<slug>/, never in a
#   worktree, and the prompt is told so. The harness creates that directory and
#   is the only thing that removes it — and it only ever removes a path it
#   built itself under /tmp/plan-125-scratch (memory:
#   test-accept-second-arg-is-rm-rf-scratch — never take a real directory as a
#   scratch argument).
#
# USAGE
#
#   doc-review-fanout.sh --letter B --units <file> --prompt <file> [options]
#   doc-review-fanout.sh --reconcile --letter B --units <file>
#   doc-review-fanout.sh --cleanup   --letter B
#
#   --jobs N        concurrent reviewers (default 6)
#   --timeout S     per-run wall-clock seconds (default 1800)
#   --dry-run       print what would run; make no worktree and no reviewer call
#
# Environment:
#   REVIEWER_MODEL  model id passed to `claude --model` (default claude-fable-5-1)
#   CLAUDE          path to the Claude Code CLI (default: `claude` on PATH)
#
# A unit is one line of the units file, e.g.
#   man-pkg:color            man-page:color/mix        man-topic:flow/if
#   spec-pkg:memory          spec-file:memory/07_runtime-helper-abi.md
#
# The prompt template is substituted with:
#   {{UNIT}}      the whole unit line              e.g. man-page:color/mix
#   {{KIND}}      the part before the first ':'    e.g. man-page
#   {{TARGET}}    the part after the first ':'     e.g. color/mix
#   {{MFB}}       absolute path to the release binary
#   {{SCRATCH}}   absolute path to this unit's scratch directory
#
# Output, all under planning/plan-125-findings/<letter>/ :
#   <slug>.md          the reviewer's final message — the findings
#   <slug>.log         the full transcript (stream-json), kept for auditing a FAILED row
#   manifest.tsv       unit  exit  seconds  findings_lines  dirty  banner

set -uo pipefail

export LC_ALL=${LC_ALL:-en_US.UTF-8}

REVIEWER_MODEL=${REVIEWER_MODEL:-claude-fable-5-1}
CLAUDE=${CLAUDE:-$(command -v claude || printf '%s' "$HOME/.local/bin/claude")}
REPO=$(git rev-parse --show-toplevel 2>/dev/null) || {
	printf 'doc-review-fanout: not in a git repository\n' >&2; exit 2; }
MFB=${MFB:-$REPO/target/release/mfb}

WT_ROOT=/tmp/plan-125-worktrees
SCRATCH_ROOT=/tmp/plan-125-scratch

LETTER=''
UNITS=''
PROMPT=''
JOBS=6
TIMEOUT=1800
MODE=run
DRYRUN=0

die() { printf 'doc-review-fanout: %s\n' "$*" >&2; exit 2; }

while [ $# -gt 0 ]; do
	case "$1" in
	--letter)    LETTER=$2; shift 2 ;;
	--units)     UNITS=$2; shift 2 ;;
	--prompt)    PROMPT=$2; shift 2 ;;
	--jobs)      JOBS=$2; shift 2 ;;
	--timeout)   TIMEOUT=$2; shift 2 ;;
	--reconcile) MODE=reconcile; shift ;;
	--cleanup)   MODE=cleanup; shift ;;
	--dry-run)   DRYRUN=1; shift ;;
	*) die "unknown option: $1" ;;
	esac
done

[ -n "$LETTER" ] || die 'need --letter'
printf '%s' "$LETTER" | grep -qE '^[A-Za-z0-9_-]+$' || die "unsafe --letter: $LETTER"

OUT="$REPO/planning/plan-125-findings/$LETTER"
MANIFEST="$OUT/manifest.tsv"

# A unit becomes a filename. Everything outside [A-Za-z0-9._-] collapses to '-',
# so `man-page:color/mix` -> `man-page-color-mix`. Slugs are checked for
# collisions before any run starts, because two units sharing an output file is
# a silently lost review.
slug() { printf '%s' "$1" | tr -c 'A-Za-z0-9._-' '-'; }

# ---------------------------------------------------------------------------
# --cleanup: remove this letter's worktrees and scratch. Only ever touches
# paths this script created, under the two /tmp roots it owns.
mode_cleanup() {
	local i wt
	for wt in "$WT_ROOT/$LETTER"-*; do
		[ -d "$wt" ] || continue
		git worktree remove --force "$wt" 2>/dev/null || rm -rf "$wt"
		printf 'removed worktree %s\n' "$wt"
	done
	if [ -d "$SCRATCH_ROOT/$LETTER" ]; then
		rm -rf "${SCRATCH_ROOT:?}/${LETTER:?}"
		printf 'removed scratch %s\n' "$SCRATCH_ROOT/$LETTER"
	fi
	git worktree prune
}

# ---------------------------------------------------------------------------
# --reconcile: the gate every letter runs before it closes.
#
# "All the reviews came back clean" is only meaningful if every unit ran, so
# this compares the unit LIST against the manifest and the findings files, and
# says which specific units are unaccounted for.
mode_reconcile() {
	[ -n "$UNITS" ] || die 'need --units'
	[ -f "$UNITS" ] || die "no units file: $UNITS"
	[ -f "$MANIFEST" ] || { printf 'RECONCILE FAIL: no manifest at %s\n' "$MANIFEST"; return 1; }

	local bad=0 total=0 unit s row
	while IFS= read -r unit; do
		case "$unit" in ''|'#'*) continue ;; esac
		total=$((total + 1))
		s=$(slug "$unit")
		# The LAST row for this unit, not the first. A re-run appends rather than
		# rewriting, so a unit that failed and was then re-run successfully has
		# two rows; reading the first one would report the stale FAILED forever
		# and there would be no way to close the letter.
		row=$(awk -F'\t' -v u="$unit" '$1 == u { last = $0 } END { print last }' "$MANIFEST")
		if [ -z "$row" ]; then
			printf 'MISSING    %s   (no manifest row)\n' "$unit"
			bad=$((bad + 1))
			continue
		fi
		case "$row" in
		*$'\t'FAILED*) printf 'FAILED     %s   %s\n' "$unit" "$row"; bad=$((bad + 1)); continue ;;
		esac
		if [ ! -s "$OUT/$s.md" ]; then
			printf 'NO-FINDINGS %s   (empty or absent %s.md)\n' "$unit" "$s"
			bad=$((bad + 1))
		fi
	done < "$UNITS"

	# The reverse direction: a manifest row for a unit the list does not contain
	# means the list changed under the run, and the extra review is not covered
	# by any letter's accounting.
	local extra=0 m
	while IFS=$'\t' read -r m _rest; do
		case "$m" in unit|''|'#'*) continue ;; esac
		grep -qxF "$m" "$UNITS" || { printf 'ORPHAN     %s   (in manifest, not in unit list)\n' "$m"; extra=$((extra + 1)); }
	done < "$MANIFEST"

	printf '\nRECONCILE letter=%s units=%s unaccounted=%s orphans=%s\n' \
		"$LETTER" "$total" "$bad" "$extra"
	[ "$bad" -eq 0 ] && [ "$extra" -eq 0 ]
}

# ---------------------------------------------------------------------------
# Wall-clock bound for one reviewer. macOS ships no coreutils `timeout`
# (`command -v timeout` is empty here), so the watchdog is spelled out: start
# the run, start a killer, and whichever finishes first decides. A timed-out
# run is reported as exit 124, the same code GNU timeout uses, so the manifest
# reads the same on either platform.
run_bounded() { # $1 = seconds, rest = command
	local secs=$1; shift
	"$@" &
	local pid=$!
	( sleep "$secs"; kill -9 "$pid" 2>/dev/null ) &
	local killer=$!
	local rc=0
	wait "$pid" 2>/dev/null || rc=$?
	local fired=0
	kill -0 "$killer" 2>/dev/null || fired=1
	kill "$killer" 2>/dev/null
	wait "$killer" 2>/dev/null
	# Two signals that the watchdog fired, and both are needed. The killer
	# having exited is the clean one; rc 137 (128+SIGKILL) is the race, where
	# the killer has sent the signal but not yet reaped itself, so `kill -0`
	# still succeeds. Without the second test that run is reported as a plain
	# crash and the timeout is invisible in the manifest.
	if [ "$fired" = 1 ] || [ "$rc" = 137 ]; then
		rc=124
	fi
	return "$rc"
}

# ---------------------------------------------------------------------------
# One unit, in one worktree. Appends exactly one manifest row, whatever happens.
run_unit() { # $1 = unit, $2 = worktree
	local unit=$1 wt=$2
	local s; s=$(slug "$unit")
	local kind=${unit%%:*}
	local target=${unit#*:}
	local scratch="$SCRATCH_ROOT/$LETTER/$s"
	local findings="$OUT/$s.md"
	local log="$OUT/$s.log"

	mkdir -p "$scratch"

	local prompt_file="$scratch/prompt.txt"
	sed -e "s|{{UNIT}}|$unit|g" \
	    -e "s|{{KIND}}|$kind|g" \
	    -e "s|{{TARGET}}|$target|g" \
	    -e "s|{{MFB}}|$MFB|g" \
	    -e "s|{{SCRATCH}}|$scratch|g" \
	    "$PROMPT" > "$prompt_file"

	# `claude` has no -C: run from the unit's worktree. run_unit is already its
	# own background subshell, so this cd changes nothing for its siblings.
	cd "$wt" || { printf '%s: cannot cd to %s\n' "$unit" "$wt" >&2; return 1; }

	local attempt rc=0 start end secs
	for attempt in 1 2; do
		rm -f "$findings"
		start=$(date +%s)
		run_bounded "$TIMEOUT" \
			"$CLAUDE" -p --model "$REVIEWER_MODEL" \
				--output-format stream-json --verbose \
				--no-session-persistence \
				--permission-mode bypassPermissions \
				--add-dir "$scratch" \
				--disallowedTools 'Bash(git commit:*)' 'Bash(git push:*)' \
				< "$prompt_file" > "$log" 2>&1
		rc=$?
		end=$(date +%s)
		secs=$((end - start))
		# The findings are the final `result` event of a successful session. A
		# session that ended in an error (usage limit, API failure) has
		# is_error=true: write nothing, so the empty-findings rule below fails it.
		jq -r 'select(.type == "result" and .is_error == false) | .result' \
			"$log" > "$findings" 2>/dev/null
		[ -s "$findings" ] || rm -f "$findings"
		# An empty findings file is a failed run wearing a success exit code:
		# the reviewer produced nothing, and a zero-byte file downstream reads
		# as "reviewed, no findings". Treat it exactly like a non-zero exit.
		if [ "$rc" -eq 0 ] && [ -s "$findings" ]; then
			break
		fi
		local got=0
		[ -f "$findings" ] && got=$(wc -c < "$findings" | tr -d ' ')
		printf '%s attempt %s: exit=%s findings=%s bytes — requeueing\n' \
			"$unit" "$attempt" "$rc" "$got" >&2
	done

	# Did the reviewer write to the repository? It is told not to; this is the
	# check that the instruction held, not an assumption that it did.
	local dirty=clean
	if [ -n "$(git -C "$wt" status --porcelain 2>/dev/null)" ]; then
		dirty=DIRTY
		git -C "$wt" reset --hard --quiet 2>/dev/null
		git -C "$wt" clean -fdq 2>/dev/null
	fi

	local banner findings_lines status
	# "Claude Code v<version>/<model>" from the session's init event — the model
	# the session actually ran, not the one requested.
	banner=$(jq -r 'select(.type == "system" and .subtype == "init")
		| "Claude Code v\(.claude_code_version)/\(.model)"' "$log" 2>/dev/null | head -1)
	banner=${banner:-unknown}
	findings_lines=$(wc -l < "$findings" 2>/dev/null | tr -d ' ')
	findings_lines=${findings_lines:-0}

	if [ "$rc" -eq 0 ] && [ -s "$findings" ]; then
		status=$rc
	else
		status=FAILED
	fi

	# One row, appended atomically enough for line-sized writes, whatever the
	# outcome was. This is the accounting the whole plan rests on.
	printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
		"$unit" "$status" "$secs" "$findings_lines" "$dirty" "$banner" >> "$MANIFEST"
}

# ---------------------------------------------------------------------------
mode_run() {
	[ -n "$UNITS" ] || die 'need --units'
	[ -f "$UNITS" ] || die "no units file: $UNITS"
	[ -n "$PROMPT" ] || die 'need --prompt'
	[ -f "$PROMPT" ] || die "no prompt file: $PROMPT"
	[ -x "$MFB" ] || die "no release binary at $MFB (cargo build --release)"
	[ "$DRYRUN" = 1 ] || [ -x "$CLAUDE" ] || die "no claude CLI at $CLAUDE"
	command -v jq >/dev/null || die "jq is required to read the reviewer transcript"

	mkdir -p "$OUT" "$WT_ROOT" "$SCRATCH_ROOT/$LETTER"

	# Collision check BEFORE anything runs: two units sharing a slug would share
	# an output file, and the second review would erase the first with no error
	# anywhere. Cheaper to refuse than to discover it in the reconciliation.
	local dupes
	dupes=$(grep -vE '^[[:space:]]*($|#)' "$UNITS" | tr -c 'A-Za-z0-9._-\n' '-' | sort | uniq -d)
	[ -z "$dupes" ] || die "unit slugs collide: $(printf '%s' "$dupes" | tr '\n' ' ')"

	[ -f "$MANIFEST" ] || printf 'unit\texit\tseconds\tfindings_lines\tdirty\tbanner\n' > "$MANIFEST"

	# N detached worktrees at the letter's base commit, created once and reused
	# round-robin. Detached on purpose: they are read surfaces, not branches.
	local base; base=$(git rev-parse HEAD)
	local i wts=()
	for i in $(seq 1 "$JOBS"); do
		local wt="$WT_ROOT/$LETTER-$i"
		if [ ! -d "$wt" ]; then
			if [ "$DRYRUN" = 1 ]; then
				printf 'DRY-RUN would create worktree %s at %s\n' "$wt" "$base"
			else
				git worktree add --detach --quiet "$wt" "$base" || die "worktree add failed: $wt"
			fi
		fi
		wts+=("$wt")
	done

	# Slot table: slot i owns worktree wts[i] and holds at most one live PID.
	#
	# NOT round-robin. Handing unit n to worktree n%JOBS looks like the same
	# thing and is not: when one review runs long, round-robin still assigns the
	# next unit to its worktree, so two reviewer processes share one working tree.
	# They would interleave probe builds and each would see the other's files in
	# `git status`, making the DIRTY check report noise and reset a live run's
	# scratch. A slot is reused only once its own PID has exited.
	local slot_pid=()
	for i in $(seq 1 "$JOBS"); do slot_pid[$((i-1))]=''; done

	local unit free
	while IFS= read -r unit; do
		case "$unit" in ''|'#'*) continue ;; esac
		if [ "$DRYRUN" = 1 ]; then
			printf 'DRY-RUN %-48s -> %s\n' "$unit" "${wts[0]}"
			continue
		fi
		free=''
		while [ -z "$free" ]; do
			for i in $(seq 0 $((JOBS - 1))); do
				if [ -z "${slot_pid[$i]}" ] || ! kill -0 "${slot_pid[$i]}" 2>/dev/null; then
					free=$i
					break
				fi
			done
			[ -z "$free" ] && sleep 2
		done
		run_unit "$unit" "${wts[$free]}" &
		slot_pid[$free]=$!
	done < "$UNITS"
	wait

	[ "$DRYRUN" = 1 ] && return 0
	printf '\n'
	mode_reconcile
}

case "$MODE" in
run)       mode_run ;;
reconcile) mode_reconcile ;;
cleanup)   mode_cleanup ;;
esac

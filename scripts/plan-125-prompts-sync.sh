#!/usr/bin/env bash
#
# plan-125-prompts-sync.sh — keep §5 of plan-125-A identical to the prompt
# files the harness actually runs.
#
# plan-125-A Phase 4 requires the reviewer prompts to exist BOTH as files
# (so `doc-review-fanout.sh --prompt` can pass them) AND verbatim in §5 of the
# plan (so a reader can see what every one of ~900 runs was asked, without
# leaving the plan), with a diff proving the two match.
#
# Two hand-maintained copies of a thousand lines is a copy that drifts — which
# is exactly what `.ai/man-content.md` §4.2 refuses to do with the banned-word
# list, and for the same reason. So there is one source (the files) and §5 is
# GENERATED from them. `--check` is the diff the plan asks for; `--write`
# regenerates §5 in place.
#
# A prompt edit mid-plan makes findings incomparable across letters and is
# recorded as a Correction — that is a policy this script cannot enforce, but
# `--check` makes the drift impossible to miss.
#
# Usage:  plan-125-prompts-sync.sh --check   (exit 1 if §5 is stale)
#         plan-125-prompts-sync.sh --write

set -uo pipefail

REPO=$(git rev-parse --show-toplevel) || exit 2
PLAN="$REPO/planning/completed/plan-125-A-standards-tooling-pilot.md"
PROMPTS="$REPO/planning/plan-125-prompts"

BEGIN='<!-- BEGIN GENERATED PROMPTS -- edit planning/plan-125-prompts/*.txt, then scripts/plan-125-prompts-sync.sh --write -->'
END='<!-- END GENERATED PROMPTS -->'

# The nine, in the order §5 presents them: the man surface's three iterations
# and its final sweep, then the spec surface's, then the cross-package
# consistency prompt plan-125-B Phase 4 added (plan-125-B C-6).
ORDER='man-iter1-package man-iter2-page man-iter3-package man-final-lens spec-iter1-package spec-iter2-file spec-iter3-package spec-final-lens man-consistency'

TITLES_man_iter1_package='5.1 Man, iteration 1 — the package or topic as a whole'
TITLES_man_iter2_page='5.2 Man, iteration 2 — one page, every sentence verified'
TITLES_man_iter3_package='5.3 Man, iteration 3 — re-integration after the page pass'
TITLES_man_final_lens='5.4 Man, final sweep — one lens over the complete manual'
TITLES_spec_iter1_package='5.5 Spec, iteration 1 — the package as a whole'
TITLES_spec_iter2_file='5.6 Spec, iteration 2 — one file, every claim verified'
TITLES_spec_iter3_package='5.7 Spec, iteration 3 — re-integration after the file pass'
TITLES_spec_final_lens='5.8 Spec, final sweep — one lens over the whole spec'
TITLES_man_consistency='5.9 Man, cross-package consistency — one dimension over the condensed manual'

title_of() {
	local key
	key=$(printf '%s' "$1" | tr '-' '_')
	eval "printf '%s' \"\$TITLES_$key\""
}

render() {
	printf '%s\n\n' "$BEGIN"
	printf 'The prompts every one of plan-125'"'"'s reviewer runs uses, verbatim.\n'
	printf 'They are generated from `planning/plan-125-prompts/*.txt`, which is what\n'
	printf '`doc-review-fanout.sh --prompt` actually passes to `codex exec`; run\n'
	printf '`./scripts/plan-125-prompts-sync.sh --check` to prove this section matches.\n\n'
	printf 'The harness substitutes `{{UNIT}}`, `{{KIND}}`, `{{TARGET}}`, `{{MFB}}` and\n'
	printf '`{{SCRATCH}}` per run; everything else is identical across all ~900 runs, so\n'
	printf 'findings stay comparable between letters.\n\n'
	local p
	for p in $ORDER; do
		printf '### %s\n\n' "$(title_of "$p")"
		printf '`planning/plan-125-prompts/%s.txt`\n\n' "$p"
		printf '```\n'
		cat "$PROMPTS/$p.txt"
		printf '```\n\n'
	done
	printf '%s\n' "$END"
}

missing=0
for p in $ORDER; do
	[ -f "$PROMPTS/$p.txt" ] || { printf 'missing prompt file: %s.txt\n' "$p" >&2; missing=1; }
done
[ "$missing" = 0 ] || exit 2

case "${1:---check}" in
--check)
	current=$(awk -v b="$BEGIN" -v e="$END" '$0==b{f=1} f{print} $0==e{f=0}' "$PLAN")
	if [ -z "$current" ]; then
		printf 'plan-125-prompts-sync: §5 has no generated block yet (run --write)\n' >&2
		exit 1
	fi
	if diff -u <(printf '%s\n' "$current") <(render) > /dev/null; then
		printf 'plan-125-prompts-sync: §5 matches all %s prompt files\n' "$(printf '%s\n' $ORDER | wc -l | tr -d ' ')"
		exit 0
	fi
	printf 'plan-125-prompts-sync: §5 is STALE:\n' >&2
	diff -u <(printf '%s\n' "$current") <(render) >&2
	exit 1
	;;
--write)
	tmp=$(mktemp -t p125prompts)
	if grep -qF "$BEGIN" "$PLAN"; then
		awk -v b="$BEGIN" -v e="$END" '
			$0==b { skip=1; print "@@GENERATED@@"; next }
			$0==e { skip=0; next }
			!skip { print }
		' "$PLAN" > "$tmp"
	else
		printf 'plan-125-prompts-sync: no generated block in §5; add the markers first\n' >&2
		exit 2
	fi
	body=$(mktemp -t p125body)
	render > "$body"
	awk -v f="$body" '
		$0=="@@GENERATED@@" { while ((getline line < f) > 0) print line; next }
		{ print }
	' "$tmp" > "$PLAN"
	rm -f "$tmp" "$body"
	printf 'plan-125-prompts-sync: §5 regenerated from %s prompt files\n' "$(printf '%s\n' $ORDER | wc -l | tr -d ' ')"
	;;
*)
	printf 'usage: plan-125-prompts-sync.sh [--check|--write]\n' >&2
	exit 2
	;;
esac

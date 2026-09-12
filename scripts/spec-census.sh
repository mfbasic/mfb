#!/usr/bin/env bash
#
# spec-census.sh — plan-125's census instrument for the embedded `mfb spec`
# contributor documentation under src/docs/spec/**.
#
# The spec is markdown embedded in the binary (src/docs/spec/mod.rs:PACKAGE_ORDER),
# and until plan-125 NOTHING measured it: not a citation, not a cross-link, not a
# stale claim. `.ai/specifications.md` states the rules; this script measures
# compliance with them.
#
# The instrument that matters is --citations. A `[[path:Symbol]]` provenance
# marker rots silently in two distinct ways, and they need different repairs:
#   * STALE BY MOVE     — the symbol still exists in src/, at another path. The
#                         link is wrong; the claim is probably still true.
#   * STALE BY DELETION — the symbol exists NOWHERE in src/. The thing the
#                         sentence describes is gone, so the CLAIM is suspect,
#                         not just the link. A fixer that only re-points paths
#                         silently ratifies this class.
# The ELSEWHERE column is what separates them, so it is never optional.
#
# Modes:
#   spec-census.sh [--fill] [pkg...]       per-package inventory (default)
#   spec-census.sh --citations [pkg...]    resolve every [[ ]] provenance marker
#   spec-census.sh --links [pkg...]        resolve every `mfb spec`/`mfb man` ref
#   spec-census.sh --render [pkg...]       render each package --all; check leaks
#   spec-census.sh --fences [pkg...]       code-fence inventory by language tag
#
# With no package arguments every spec package is censused, in PACKAGE_ORDER.
# Output is deterministic: no timestamps, no wall-clock, stable ordering.
#
# Env: MFB (default ./target/release/mfb), SPEC (default src/docs/spec)

set -uo pipefail

MFB=${MFB:-./target/release/mfb}
SPEC=${SPEC:-src/docs/spec}

# Rendered spec output is UTF-8; a C locale makes multibyte regexes match bytes.
export LC_ALL=${LC_ALL:-en_US.UTF-8}

# The reading order from src/docs/spec/mod.rs:PACKAGE_ORDER. Kept here so the
# census output orders like the rendered manual does rather than alphabetically;
# any package on disk but absent from this list is appended in sorted order, the
# same graceful degradation the renderer itself does.
PACKAGE_ORDER='architecture language memory linker threading package diagnostics tooling package-manager unicode app stdlib'

die() { printf 'spec-census: %s\n' "$*" >&2; exit 2; }

# ---------------------------------------------------------------------------
# Package list resolution.
packages() {
	local want=("$@")
	local all=()
	local p q seen
	for p in $PACKAGE_ORDER; do
		[ -d "$SPEC/$p" ] && all+=("$p")
	done
	# anything on disk but not in PACKAGE_ORDER, appended in sorted order
	for p in $(ls -1 "$SPEC" 2>/dev/null | sort); do
		[ -d "$SPEC/$p" ] || continue
		seen=0
		for q in $PACKAGE_ORDER; do [ "$p" = "$q" ] && seen=1; done
		[ "$seen" = 0 ] && all+=("$p")
	done
	if [ ${#want[@]} -eq 0 ]; then
		printf '%s\n' "${all[@]}"
		return
	fi
	for p in "${want[@]}"; do
		[ -d "$SPEC/$p" ] || die "unknown spec package: $p"
		printf '%s\n' "$p"
	done
}

# ---------------------------------------------------------------------------
# --fill: the denominator every spec letter reconciles against.
mode_fill() {
	local pkgs
	pkgs=$(packages "$@") || exit 2
	printf '%-18s %6s %8s %9s %7s %6s %6s\n' PACKAGE FILES LINES WORDS FENCES CITES LINKS
	local tf=0 tl=0 tw=0 tfe=0 tc=0 tk=0
	local p files lines words fences cites links
	for p in $pkgs; do
		files=$(find "$SPEC/$p" -name '*.md' | wc -l | tr -d ' ')
		lines=$(find "$SPEC/$p" -name '*.md' -exec cat {} + | wc -l | tr -d ' ')
		words=$(find "$SPEC/$p" -name '*.md' -exec cat {} + | wc -w | tr -d ' ')
		fences=$(find "$SPEC/$p" -name '*.md' -exec cat {} + | grep -c '^```')
		fences=$(( fences / 2 ))
		cites=$(find "$SPEC/$p" -name '*.md' -exec cat {} + | grep -oE '\[\[[^]]+\]\]' | wc -l | tr -d ' ')
		links=$(find "$SPEC/$p" -name '*.md' -exec cat {} + | grep -oE 'mfb (spec|man) [a-zA-Z-]+' | wc -l | tr -d ' ')
		printf '%-18s %6s %8s %9s %7s %6s %6s\n' "$p" "$files" "$lines" "$words" "$fences" "$cites" "$links"
		tf=$((tf+files)); tl=$((tl+lines)); tw=$((tw+words))
		tfe=$((tfe+fences)); tc=$((tc+cites)); tk=$((tk+links))
	done
	printf '%-18s %6s %8s %9s %7s %6s %6s\n' TOTAL "$tf" "$tl" "$tw" "$tfe" "$tc" "$tk"
}

# ---------------------------------------------------------------------------
# Citation resolution.
#
# A marker is [[BODY]]. Split BODY on the FIRST ':':
#   * no ':'            -> BODY is a path. (A symbol written with no path is
#                          malformed and lands here as MISS-PATH, which is the
#                          honest verdict: there is no path to resolve.)
#   * suffix all digits
#     or N-M            -> line / line-range citation; check it is within the file.
#   * otherwise         -> symbol citation; grep -F the symbol in the cited file.
#
# FIRST, not last: a Rust symbol legitimately contains '::'
# ([[src/codegen/engine/value/builder_values.rs:NirValue::FunctionRef]]), and a
# last-colon split shreds it into a nonexistent path plus a bare `FunctionRef`,
# reporting MISS-PATH on a citation that is perfectly fine. No path anywhere in
# this tree contains ':', so the first colon is always the path/suffix seam.
cite_path() { printf '%s' "${1%%:*}"; }
cite_suffix() {
	case "$1" in
		*:*) printf '%s' "${1#*:}" ;;
		*)   printf '' ;;
	esac
}

# Does a symbol appear anywhere under the tracked source roots? This is the
# column that separates stale-by-move from stale-by-deletion.
symbol_exists_anywhere() {
	local sym=$1
	if grep -rqF --include='*.rs' -e "$sym" src build.rs repository/src 2>/dev/null; then
		printf 'move'
	else
		printf 'deleted'
	fi
}

mode_citations() {
	local pkgs
	pkgs=$(packages "$@") || exit 2
	local roots=()
	local p
	for p in $pkgs; do roots+=("$SPEC/$p"); done

	# occurrence list: citation<TAB>specfile<TAB>line
	local occ uniq
	occ=$(mktemp -t speccite)
	find "${roots[@]}" -name '*.md' -print0 \
		| xargs -0 grep -noE '\[\[[^]]+\]\]' 2>/dev/null \
		| sed -E 's/^([^:]+):([0-9]+):(.*)$/\3	\1	\2/' \
		| sort > "$occ"

	uniq=$(mktemp -t specuniq)
	cut -f1 "$occ" | sort -u > "$uniq"

	local n_uniq n_nosuffix=0 n_lin=0 n_sym=0
	local n_misspath=0 n_missline=0 n_misssym=0 n_move=0 n_deleted=0
	n_uniq=$(wc -l < "$uniq" | tr -d ' ')

	local marker body path suffix verdict extra hi total f l
	while IFS= read -r marker; do
		body=${marker#\[\[}
		body=${body%\]\]}
		path=$(cite_path "$body")
		suffix=$(cite_suffix "$body")
		verdict=OK
		extra=''

		if [ -z "$suffix" ]; then
			n_nosuffix=$((n_nosuffix+1))
			[ -e "$path" ] || verdict=MISS-PATH
		elif printf '%s' "$suffix" | grep -qE '^[0-9]+(-[0-9]+)?$'; then
			n_lin=$((n_lin+1))
			if [ ! -f "$path" ]; then
				verdict=MISS-PATH
			else
				hi=${suffix##*-}
				total=$(wc -l < "$path" | tr -d ' ')
				if [ "$hi" -gt "$total" ]; then
					verdict=MISS-LINE
					extra="file has $total lines"
				fi
			fi
		else
			n_sym=$((n_sym+1))
			if [ ! -e "$path" ]; then
				verdict=MISS-PATH
			elif [ -d "$path" ]; then
				grep -rqF -e "$suffix" "$path" 2>/dev/null || verdict=MISS-SYMBOL
			else
				grep -qF -e "$suffix" "$path" 2>/dev/null || verdict=MISS-SYMBOL
			fi
			if [ "$verdict" = MISS-SYMBOL ]; then
				extra=$(symbol_exists_anywhere "$suffix")
				case "$extra" in
					move)    n_move=$((n_move+1));    extra='ELSEWHERE=yes  STALE-BY-MOVE' ;;
					deleted) n_deleted=$((n_deleted+1)); extra='ELSEWHERE=no   STALE-BY-DELETION (claim suspect)' ;;
				esac
			fi
		fi

		case "$verdict" in
			MISS-PATH)   n_misspath=$((n_misspath+1)) ;;
			MISS-LINE)   n_missline=$((n_missline+1)) ;;
			MISS-SYMBOL) n_misssym=$((n_misssym+1)) ;;
		esac

		[ "$verdict" = OK ] && continue

		# every spec site that wrote this citation, so a finding is actionable
		while IFS='	' read -r _m f l; do
			printf '%-12s %-46s %-64s %s\n' "$verdict" "$f:$l" "$marker" "$extra"
		done < <(awk -F'	' -v m="$marker" '$1==m {print}' "$occ")
	done < "$uniq"

	printf '\n'
	printf 'TOTAL unique=%s  nosuffix=%s  line=%s  symbol=%s\n' "$n_uniq" "$n_nosuffix" "$n_lin" "$n_sym"
	printf 'MISS-PATH %s\n' "$n_misspath"
	printf 'MISS-LINE %s\n' "$n_missline"
	printf 'MISS-SYMBOL %s  (stale-by-move %s, stale-by-deletion %s)\n' "$n_misssym" "$n_move" "$n_deleted"
	rm -f "$occ" "$uniq"
	[ "$((n_misspath+n_missline+n_misssym))" -eq 0 ]
}

# ---------------------------------------------------------------------------
# --links: every `mfb spec <pkg> [<topic>]` and `mfb man <pkg>` reference in the
# prose must resolve. A dead cross-link is a dead end for the reader and no test
# has ever checked one.
mode_links() {
	local pkgs
	pkgs=$(packages "$@") || exit 2
	local roots=()
	local p
	for p in $pkgs; do roots+=("$SPEC/$p"); done
	local bad=0 total=0
	local rec f rest l text kind target
	while IFS= read -r rec; do
		f=${rec%%:*}; rest=${rec#*:}
		l=${rest%%:*}; text=${rest#*:}
		kind=$(printf '%s' "$text" | awk '{print $2}')
		target=$(printf '%s' "$text" | awk '{print $3}')
		[ -n "$target" ] || continue
		total=$((total+1))
		if [ "$kind" = spec ]; then
			[ -d "$SPEC/$target" ] || { printf '%-10s %-46s %s\n' MISS-SPEC "$f:$l" "$text"; bad=$((bad+1)); }
		else
			"$MFB" man "$target" >/dev/null 2>&1 \
				|| { printf '%-10s %-46s %s\n' MISS-MAN "$f:$l" "$text"; bad=$((bad+1)); }
		fi
	done < <(find "${roots[@]}" -name '*.md' -print0 | xargs -0 grep -noE 'mfb (spec|man) [a-zA-Z-]+' 2>/dev/null | sort -u)
	printf '\nTOTAL links=%s  unresolved=%s\n' "$total" "$bad"
	[ "$bad" -eq 0 ]
}

# ---------------------------------------------------------------------------
# --render: the renderer strips [[ ]] markers, so any that survives into rendered
# output is malformed. An empty render is a package the reader cannot reach.
mode_render() {
	local pkgs
	pkgs=$(packages "$@") || exit 2
	local bad=0
	local p out lines leaks
	printf '%-18s %8s %7s\n' PACKAGE LINES LEAKS
	for p in $pkgs; do
		out=$("$MFB" spec "$p" --all 2>/dev/null)
		lines=$(printf '%s\n' "$out" | wc -l | tr -d ' ')
		leaks=$(printf '%s\n' "$out" | grep -c '\[\[')
		printf '%-18s %8s %7s\n' "$p" "$lines" "$leaks"
		if [ "$lines" -le 1 ]; then
			printf '%-18s EMPTY RENDER\n' "$p"
			bad=$((bad+1))
		fi
		[ "$leaks" -gt 0 ] && bad=$((bad+1))
	done
	[ "$bad" -eq 0 ]
}

# ---------------------------------------------------------------------------
# --fences: which of the code fences are MFBASIC, and therefore compilable. An
# untagged fence is not a defect on its own; it is a fence nobody can machine-check.
mode_fences() {
	local pkgs
	pkgs=$(packages "$@") || exit 2
	local roots=()
	local p
	for p in $pkgs; do roots+=("$SPEC/$p"); done
	printf '%-16s %6s\n' LANG COUNT
	find "${roots[@]}" -name '*.md' -exec cat {} + \
		| awk '/^```/ { if (inb) { inb=0; next } inb=1; t=substr($0,4); gsub(/[ \t\r]/,"",t); if (t=="") t="(untagged)"; print t }' \
		| sort | uniq -c | sort -rn \
		| awk '{printf "%-16s %6s\n", $2, $1}'
}

# ---------------------------------------------------------------------------
mode=--fill
case "${1:-}" in
	--fill|--citations|--links|--render|--fences) mode=$1; shift ;;
	--*) die "unknown mode: $1" ;;
esac

case "$mode" in
	--fill)      mode_fill "$@" ;;
	--citations) mode_citations "$@" ;;
	--links)     mode_links "$@" ;;
	--render)    mode_render "$@" ;;
	--fences)    mode_fences "$@" ;;
esac

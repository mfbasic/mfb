#!/usr/bin/env bash
#
# man-census.sh — plan-108's census instrument for `mfb man` builtin content.
#
# The `mfb man` renderer omits every empty prose section (src/cli/man.rs:473,
# 501, 509 gate on `intro/desc/example.is_empty()`), so RENDERED OUTPUT — not a
# source grep — is the honest measure of what a developer actually sees. Source
# greps both over- and under-count: `fs` shows 37 `owns` hits in Rust module
# docs and 0 rendered lines. Everything here therefore runs `mfb man` and reads
# what comes back.
#
# Modes:
#   man-census.sh [--fill] [pkg...]     per-package fill table (default)
#   man-census.sh --functions [pkg...]  per-function fill rows (finds stragglers)
#   man-census.sh --memory-scope [pkg...]
#                                       rendered hits for the plan-108-A §3 (2a)
#                                       banned memory vocabulary, with the
#                                       datetime arithmetic-borrow carve-out
#                                       classified separately
#   man-census.sh --banned-list         print the canonical banned-word list
#
# With no package arguments every registry package is censused, in sorted order.
# Output is deterministic: no timestamps, no paths, stable ordering.
#
# Env: MFB (default ./target/release/mfb), BUILTINS (default src/codegen/builtins)

set -uo pipefail

MFB=${MFB:-./target/release/mfb}
BUILTINS=${BUILTINS:-src/codegen/builtins}

# Rendered output is UTF-8 box drawing; a C locale makes multibyte regexes
# match bytes and silently returns nothing.
export LC_ALL=${LC_ALL:-en_US.UTF-8}

# ---------------------------------------------------------------------------
# The canonical banned memory vocabulary (plan-108-A §3 (2a)).
#
# THIS IS THE ONE SOURCE. `.ai/man-content.md` quotes it; `--banned-list`
# prints it so the doc can be diffed against the script rather than drifting
# from it. Permitted, and deliberately absent here: copy, mutate, value, alias.
#
# Word-sense notes baked into the patterns:
#   - `move`/`drop`/`free` have common non-memory senses, so they are matched
#     in their memory constructions only.
#   - `reference` is matched as "by reference"/"reference count" only; a
#     cross-reference to another function is not a memory claim.
#   - bare `own` is NOT banned: "builds its own copy" is the rewrite table's
#     own prescribed replacement. Only `owns`/`owned`/`owner`/`ownership` are.
#
# Matching is whole-word (see banned_regex): without boundaries `cheap` matches
# `heap` — five rendered lines did exactly that in the first run.
BANNED_CORE='borrow|borrows|borrowed|borrowing|pointer|pointers|ownership|owns|owned|owner|owners|move semantics|moved into|moves the value|consume|consumes|consumed|consuming|free the|free its|frees|freed|heap|refcount|reference count|reference-counted|garbage collect|garbage collected|lifetime|lifetimes|dangling|deep copy|shallow copy|by reference|by value|RAII|escape analysis|lexical drop|drop the value|drop the handle|allocate|allocates|allocated|allocating|allocation|allocations|allocator'

# Whole-word wrapper. BSD grep has no portable `\b`, so the boundaries are
# spelled out as "not a letter" on each side.
banned_regex() {
	printf '(^|[^A-Za-z])(%s)([^A-Za-z]|$)' "$BANNED_CORE"
}

usage() {
	sed -n '3,26p' "$0" | sed 's/^# \{0,1\}//'
}

# The censusable package set.
#
# `ls src/codegen/builtins` gives 30 directories, but that is NOT the man
# surface, for two reasons the first census run got wrong:
#
#   * `errorcode/` renders as `errorCode` — the import name is camelCase, so
#     `mfb man errorcode` errors out and the directory name censuses as 0
#     pages. It exports constants only (no callables), so 0 FUNCTION pages is
#     correct for it; its overview and description are real and in scope.
#   * `perf/` is not an MFB package at all. It is the `--cfg perf`
#     compiler-injected timing helpers (perf/perf.rs:1-6: "These are NOT an MFB
#     `perf::` package — there is no language surface"), so `mfb man perf`
#     correctly errors. It is excluded here and owned by no letter.
#
# `general` and `testing` are the reverse case: real registry packages that the
# `mfb man` index deliberately omits (their members are unqualified globals
# needing no IMPORT), but `mfb man general` renders. They ARE in scope.
packages() {
	ls "$BUILTINS" | grep -v '^mod\.rs$' | grep -v '^perf$' | sed 's/^errorcode$/errorCode/' | sort
}

# The function names a package's overview page lists. Continuation rows of a
# wrapped summary start with a blank first column, so anchoring on `^│ pkg::`
# counts each function exactly once.
functions_of() {
	local pkg=$1
	"$MFB" man "$pkg" 2>/dev/null |
		grep -oE "^│ ${pkg}::[A-Za-z0-9_]+" |
		sed "s/^│ ${pkg}:://" |
		sort -u
}

# Does this package render a types page at all? (`has_public_types`, man.rs:342)
has_types_page() {
	local pkg=$1
	"$MFB" man "$pkg" types 2>/dev/null | grep -qE '^(Records|Unions|Enums|Resources)$'
}

# ---------------------------------------------------------------------------
# One function page -> "intro desc example paramTotal paramFilled"
#
# intro   — a non-blank line between the `═` title underline and `Package`
# desc    — a `Description` section heading is present
# example — an `Examples` section heading is present
# params  — the Parameters table's Description cells, located by the header row
#           (an Aliases column shifts them, so the column index is read, never
#           assumed)
page_fill() {
	local pkg=$1 fn=$2
	"$MFB" man "$pkg" "$fn" 2>/dev/null | awk '
		BEGIN { FS = "│"; intro = 0; desc = 0; ex = 0; ptot = 0; pfill = 0
		        seen_underline = 0; before_package = 0; dcol = 0 }

		# The title underline is line 2; anything non-blank between it and the
		# "Package" heading is the function intro.
		NR == 2 && /^═+$/ { seen_underline = 1; before_package = 1; next }
		before_package && /^Package$/ { before_package = 0; next }
		before_package { if ($0 ~ /[^ \t]/) intro = 1; next }

		/^Description$/ { desc = 1; next }
		/^Examples$/    { ex = 1; next }

		/^Parameters$/  { inparams = 1; next }
		inparams && /^┌/ { intable = 1; header = 0; next }
		inparams && intable && /^├/ { next }
		inparams && intable && /^└/ { intable = 0; inparams = 0; next }
		inparams && intable {
			if (header == 0) {
				for (i = 2; i < NF; i++) {
					h = $i; gsub(/^[ \t]+|[ \t]+$/, "", h)
					if (h == "Description") dcol = i
				}
				header = 1
				next
			}
			p = $2; gsub(/^[ \t]+|[ \t]+$/, "", p)
			if (p != "") {
				ptot++
				d = (dcol > 0 && dcol <= NF) ? $dcol : ""
				gsub(/^[ \t]+|[ \t]+$/, "", d)
				if (d != "") pfill++
			}
		}

		END { print intro, desc, ex, ptot, pfill }
	'
}

# ---------------------------------------------------------------------------
# One types page -> "described/total", where an entry is a record FIELD, a
# union or enum VARIANT, or a resource. Variants render as "• Name — desc"
# (man.rs:371,391) and the em dash is emitted whether or not a description
# follows, so the text AFTER it is what decides.
types_fill() {
	local pkg=$1
	"$MFB" man "$pkg" types 2>/dev/null | awk '
		BEGIN { FS = "│"; tot = 0; fill = 0; sec = "" }

		/^Records$/    { sec = "rec"; next }
		/^Unions$/     { sec = "uni"; next }
		/^Enums$/      { sec = "enum"; next }
		/^Resources$/  { sec = "res"; next }

		# Record field tables.
		/^┌/ { intable = 1; header = 0; dcol = 0; next }
		/^├/ { next }
		/^└/ { intable = 0; next }
		intable {
			if (header == 0) {
				for (i = 2; i < NF; i++) {
					h = $i; gsub(/^[ \t]+|[ \t]+$/, "", h)
					if (h == "Description") dcol = i
				}
				header = 1
				next
			}
			f = $2; gsub(/^[ \t]+|[ \t]+$/, "", f)
			if (f != "") {
				tot++
				d = (dcol > 0 && dcol <= NF) ? $dcol : ""
				gsub(/^[ \t]+|[ \t]+$/, "", d)
				if (d != "") fill++
			}
			next
		}

		# Union / enum variants.
		/^[ \t]*•[ \t]/ {
			tot++
			line = $0
			if (sub(/^[^—]*—[ \t]*/, "", line) && line ~ /[^ \t]/) fill++
			pending = 0
			next
		}

		# A resource is a "pkg::Name" heading followed by free prose.
		sec == "res" && /^[a-z_][a-z_0-9]*::[A-Za-z0-9_]+$/ { pending = 1; tot++; next }
		pending && /[^ \t]/ { fill++; pending = 0; next }

		END { printf "%d/%d", fill, tot }
	'
}

# ---------------------------------------------------------------------------
mode_functions() {
	local pkgs=("$@")
	printf '%-14s %-24s %5s %5s %5s %s\n' PACKAGE FUNCTION INTRO DESC EXMPL PARAM-DESC
	local pkg fn row
	for pkg in "${pkgs[@]}"; do
		for fn in $(functions_of "$pkg"); do
			row=$(page_fill "$pkg" "$fn")
			set -- $row
			printf '%-14s %-24s %5s %5s %5s %d/%d\n' \
				"$pkg" "$fn" \
				"$([ "$1" = 1 ] && echo yes || echo NO)" \
				"$([ "$2" = 1 ] && echo yes || echo NO)" \
				"$([ "$3" = 1 ] && echo yes || echo NO)" \
				"$5" "$4"
		done
	done
}

mode_fill() {
	local pkgs=("$@")
	local pkg fn row
	local t_fn=0 t_intro=0 t_desc=0 t_ex=0 t_ptot=0 t_pfill=0 t_none=0

	printf '%-14s %5s %6s %6s %8s %11s %6s %6s\n' \
		PACKAGE PAGES INTRO DESC EXAMPLE PARAM-DESC PKGDOC TYPES
	printf '%s\n' '---------------------------------------------------------------------------'

	for pkg in "${pkgs[@]}"; do
		local n=0 intro=0 desc=0 ex=0 ptot=0 pfill=0 none=0
		for fn in $(functions_of "$pkg"); do
			row=$(page_fill "$pkg" "$fn")
			set -- $row
			n=$((n + 1))
			intro=$((intro + $1))
			desc=$((desc + $2))
			ex=$((ex + $3))
			ptot=$((ptot + $4))
			pfill=$((pfill + $5))
			[ "$2" = 0 ] && [ "$3" = 0 ] && none=$((none + 1))
		done

		# Package overview: intro is the line under the title, desc the
		# `Description` section — both gated by man.rs:286,290.
		local pkgdoc="-"
		local ov
		ov=$("$MFB" man "$pkg" 2>/dev/null)
		local ov_intro=0 ov_desc=0
		printf '%s\n' "$ov" | sed -n '3,6p' | grep -qE '[^ ]' && ov_intro=1
		printf '%s\n' "$ov" | grep -qE '^Description$' && ov_desc=1
		pkgdoc="${ov_intro}${ov_desc}"

		# Types page: fraction of described type entries. Each countable entry
		# is one record FIELD, one union/enum VARIANT, or one resource — an
		# enum whose variants are all bare must not read as "described"
		# because its "An enum of:" line is non-blank.
		local types="-"
		if has_types_page "$pkg"; then
			types=$(types_fill "$pkg")
		fi

		printf '%-14s %5d %6d %6d %8d %11s %6s %6s\n' \
			"$pkg" "$n" "$intro" "$desc" "$ex" "$pfill/$ptot" "$pkgdoc" "$types"

		t_fn=$((t_fn + n)); t_intro=$((t_intro + intro)); t_desc=$((t_desc + desc))
		t_ex=$((t_ex + ex)); t_ptot=$((t_ptot + ptot)); t_pfill=$((t_pfill + pfill))
		t_none=$((t_none + none))
	done

	printf '%s\n' '---------------------------------------------------------------------------'
	printf '%-14s %5d %6d %6d %8d %11s\n' \
		TOTAL "$t_fn" "$t_intro" "$t_desc" "$t_ex" "$t_pfill/$t_ptot"
	printf 'pages with neither Description nor Examples: %d\n' "$t_none"
	printf 'PKGDOC column: <overview-intro><overview-desc>, 1 = present\n'
}

# ---------------------------------------------------------------------------
# Rendered hits for the banned memory vocabulary, attributed to the page they
# render on. Carve-out 1 (plan-108-A §3 (2a)): every `borrow` in `datetime` is
# ARITHMETIC borrow ("a negative nanos value borrows a second"), not a memory
# claim — classified and counted separately, never silently dropped.
mode_memory_scope() {
	local pkgs=("$@")
	local pkg fn hits carve=0 unclassified=0

	scan_page() { # $1 = label, $2 = rendered text
		local label=$1 text=$2 line
		while IFS= read -r line; do
			local n=${line%%:*}
			local body=${line#*:}
			if [ "$pkg" = datetime ] && printf '%s' "$body" | grep -qiE 'borrow'; then
				carve=$((carve + 1))
				printf 'CARVE-1  %-28s %5s  %s\n' "$label" "$n" "$body"
			else
				unclassified=$((unclassified + 1))
				printf 'HIT      %-28s %5s  %s\n' "$label" "$n" "$body"
			fi
		done < <(printf '%s\n' "$text" | grep -niE "$(banned_regex)")
	}

	for pkg in "${pkgs[@]}"; do
		scan_page "$pkg (overview)" "$("$MFB" man "$pkg" 2>/dev/null)"
		if has_types_page "$pkg"; then
			scan_page "$pkg (types)" "$("$MFB" man "$pkg" types 2>/dev/null)"
		fi
		for fn in $(functions_of "$pkg"); do
			scan_page "$pkg::$fn" "$("$MFB" man "$pkg" "$fn" 2>/dev/null)"
		done
	done

	printf '\n'
	printf 'unclassified memory-vocabulary hits: %d\n' "$unclassified"
	printf 'carve-out 1 (datetime arithmetic borrow): %d\n' "$carve"
	[ "$unclassified" -eq 0 ]
}

# ---------------------------------------------------------------------------
main() {
	local mode=fill
	case "${1:-}" in
	--fill) mode=fill; shift ;;
	--functions) mode=functions; shift ;;
	--memory-scope) mode=memory-scope; shift ;;
	--banned-list) printf '%s\n' "$BANNED_CORE"; return 0 ;;
	-h | --help) usage; return 0 ;;
	esac

	if [ ! -x "$MFB" ]; then
		echo "man-census.sh: no mfb binary at $MFB (cargo build --release)" >&2
		return 2
	fi

	local pkgs
	if [ "$#" -gt 0 ]; then
		pkgs=("$@")
	else
		# shellcheck disable=SC2207
		pkgs=($(packages))
	fi

	case "$mode" in
	fill) mode_fill "${pkgs[@]}" ;;
	functions) mode_functions "${pkgs[@]}" ;;
	memory-scope) mode_memory_scope "${pkgs[@]}" ;;
	esac
}

main "$@"

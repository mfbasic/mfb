#!/usr/bin/env bash
#
# man-manual.sh — emit the COMPLETE `mfb man` developer manual as one
# deterministic artifact.
#
# WHY THIS EXISTS, and why letters H/G of plan-125 must not gate on raw
# `mfb man --all`:
#
#   `mfb man --all` walks the registry and, by a deliberate and documented
#   filter (src/cli/man.rs:render_all_markdown), skips every
#   `unqualified_global` package — `testing` (12 function pages) and `general`
#   (18) — because they have no writable `IMPORT <pkg>` spelling and rendering
#   a `# testing` header would advertise a spelling a developer cannot type.
#   That filter is correct for the product and wrong for an audit: those 30
#   pages ARE part of the surface a developer reads, they are simply reached by
#   name rather than by import.
#
#   Plan-125 additionally taught `--all` to render the 10 narrative guide
#   topics (32 pages), so THOSE are no longer missing from `--all`. This script
#   therefore adds exactly the two filtered packages.
#
# The artifact is deterministic: no timestamps, no wall-clock, no paths that
# vary by checkout, fixed ordering (the renderer's own order, then `general`,
# then `testing`). Two runs at one commit are byte-identical, so a diff between
# runs is a real content change.
#
# Usage:  man-manual.sh            emit the manual on stdout
#         man-manual.sh --count    print the page-count reconciliation instead
#
# Env: MFB (default ./target/release/mfb)

set -uo pipefail

MFB=${MFB:-./target/release/mfb}
BUILTINS=${BUILTINS:-src/codegen/builtins}
export LC_ALL=${LC_ALL:-en_US.UTF-8}

[ -x "$MFB" ] || { printf 'man-manual: no mfb binary at %s\n' "$MFB" >&2; exit 2; }

# The renderer's own section separator, so the seams this script makes look like
# the seams the renderer makes and no lens reads them as a formatting change.
rule() { printf '\n---\n\n'; }

# `--all` already covers every importable package and every guide topic. The two
# unqualified-global packages are appended in a fixed order.
FILTERED_PACKAGES='general testing'

emit() {
	"$MFB" man --all
	local pkg
	for pkg in $FILTERED_PACKAGES; do
		rule
		"$MFB" man "$pkg" --all
	done
}

# ---------------------------------------------------------------------------
# --count: the reconciliation letters G/H run. Every number here is read out of
# the artifact itself, never assumed.
#
# The artifact is RENDERED output, not markdown: `mfb man` prints a title in
# upper case with a U+2550 `═` rule under it for a level-1 section and a U+2500
# `─` rule for a level-2 one. Counting `═` rules is therefore an exact page
# count and it does not depend on parsing titles — which is the trap memory
# `man-page-count-scrape-overcounts` records, where a title-shaped line inside
# prose inflates the count. Every page (a package overview, a function, a types
# page, a guide topic, a guide subtopic) is exactly one `═` rule.
#
# The count is cross-checked against man-census.sh's independent denominator,
# so agreement between two instruments — not one scrape — is what closes it.
mode_count() {
	local art
	art=$(mktemp -t manmanual)
	emit > "$art"

	local lines pages
	lines=$(wc -l < "$art" | tr -d ' ')
	pages=$(grep -c '^═' "$art")

	printf 'ARTIFACT lines          %s\n' "$lines"
	printf 'PAGES (═ rules)         %s\n' "$pages"
	printf '\n'
	printf 'GENERAL present         %s\n' "$(grep -cx 'GENERAL' "$art")"
	printf 'TESTING present         %s\n' "$(grep -cx 'TESTING' "$art")"

	# Guide pages, measured two independent ways so agreement — not one scrape —
	# is what closes the count. A title scrape cannot do this: the `types` guide
	# topic renders the title `TYPES`, and so does every package's types page, so
	# `grep -cx TYPES` says 22 and means nothing.
	#
	#   (a) each topic rendered on its own, pages summed
	#   (b) the artifact's page total minus every package's own page total
	local topic pkg sum=0 pkgsum=0 n
	printf '\nguide topic pages (rendered individually)\n'
	for topic in errors flow lambda link optimizations tooling tour types unicode variable; do
		n=$("$MFB" man "$topic" --all 2>/dev/null | grep -c '^═')
		printf '  %-16s %s\n' "$topic" "$n"
		sum=$((sum+n))
	done

	# The same package denominator man-census.sh uses, and for the same reasons:
	# `perf/` and `tests/` are Rust-only directories with no man surface, and
	# `errorcode/` is importable only under its camelCase name.
	for pkg in $(ls "$BUILTINS" | grep -vE '^(mod\.rs|perf|tests)$' | sed 's/^errorcode$/errorCode/'); do
		n=$("$MFB" man "$pkg" --all 2>/dev/null | grep -c '^═')
		pkgsum=$((pkgsum+n))
	done

	printf '\nguide pages, summed per topic      %s\n' "$sum"
	printf 'guide pages, artifact minus pkgs   %s\n' "$((pages - pkgsum))"
	printf 'registry pages (31 packages)       %s\n' "$pkgsum"
	rm -f "$art"
	[ "$sum" -eq "$((pages - pkgsum))" ]
}

# ---------------------------------------------------------------------------
# --condensed: the artifact plan-125-B Phase 4's cross-package consistency
# review reads. The whole manual is ~61,000 lines and cannot be read with useful
# attention; the part where vocabulary is ESTABLISHED is much smaller — every
# package overview, every package `types` page, and every guide-topic overview.
# Function pages borrow that vocabulary, so a consistency defect shows here
# first.
#
# Fixed order, no timestamps: the packages in `ls` order (the same denominator
# man-census.sh and --count use), each overview followed by its types page when
# it has one, then the ten guide-topic overviews. A package with no types page
# prints an error from `mfb man <pkg> types`; that error is discarded and the
# page simply omitted.
mode_condensed() {
	local pkg topic types
	# Directories only: `ls` also lists loose files such as `float_result.rs`,
	# which `mfb man` rejects as an unknown package.
	for pkg in $(cd "$BUILTINS" && ls -d */ | tr -d / | grep -vE '^(perf|tests)$' | sed 's/^errorcode$/errorCode/'); do
		"$MFB" man "$pkg"
		rule
		if types=$("$MFB" man "$pkg" types 2>/dev/null); then
			printf '%s\n' "$types"
			rule
		fi
	done
	for topic in errors flow lambda link optimizations tooling tour types unicode variable; do
		"$MFB" man "$topic"
		rule
	done
}

case "${1:-}" in
	'')          emit ;;
	--count)     mode_count ;;
	--condensed) mode_condensed ;;
	*)        printf 'man-manual: unknown option: %s\n' "$1" >&2; exit 2 ;;
esac

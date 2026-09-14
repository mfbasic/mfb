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
#   man-census.sh --scope [pkg...]      rendered hits for the compiler-internals
#                                       vocabulary the standard forbids (bug/plan
#                                       numbers, mangled symbols, codegen terms)
#   man-census.sh --topics [topic...]   per-topic inventory for the narrative
#                                       guide topics under src/docs/man/**
#   man-census.sh --banned-list         print the canonical banned-word list
#
# A whole-surface --memory-scope / --scope run (no package arguments) sweeps the
# guide topics too; a run scoped to named packages does not.
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

# ---------------------------------------------------------------------------
# The compiler-internals vocabulary `.ai/man-content.md` §3 forbids: a man page
# is developer documentation, so registry/lowering/codegen machinery, mangled
# symbols, Rust items, old_man citation markers, and — the one this was added
# for — plan and bug NUMBERS have no business rendering.
#
# Kept separate from BANNED_CORE because the two answer different questions and
# every letter drives them to 0 independently. `--scope` reports this one.
# Compiler/runtime internals a developer never sees. The second line was added
# by plan-108-F: the first list caught mangled symbols and compiler nouns, but
# not the *host-API* and *mechanism* vocabulary that turned out to be the more
# common leak — "Internally the function opens the file read-only, seeks to the
# end…", "an `EINTR` interruption retries with the cursor unchanged", "copied
# into an arena-backed String", "the per-execution-context field held in the
# arena state". 42 such lines survived every earlier sweep.
SCOPE_CORE='bug-[0-9]+|plan-[0-9]+|audit-[0-9]+|abi_inline|abi_function|Body::|monomorph|NIR|\.ncode|__[a-z]+_[A-Za-z]|#[a-z]+_[A-Za-z]|\[\[[A-Za-z_][A-Za-z_/.]*:|RegistryFunction|RegistryPackage|rewrite_target|resolve_call|lowering|desugar|regalloc|vreg|codegen'
SCOPE_CORE=$SCOPE_CORE'|Internally|under the hood|arena|EINTR|EIO|EAGAIN|EWOULDBLOCK|[^A-Za-z]errno|file descriptor|the descriptor|isatty|per-execution-context|scratch buffer|[^A-Z]EEXIST|[^A-Z]ENOENT|[^A-Z]ENOTDIR|the compiler emits|IR lowering|lowered inline|lowered to a|is lowered|discriminant|runtime helper|state region'

scope_regex() {
	printf '%s' "$SCOPE_CORE"
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
#   * `perf/` is not an MFB package at all. It is the `--debug`
#     compiler-injected timing helpers (perf/perf.rs:1-6: "These are NOT an MFB
#     `perf::` package — there is no language surface"), so `mfb man perf`
#     correctly errors. It is excluded here and owned by no letter.
#   * `tests/` is the same class as `perf/`: a Rust `#[cfg(test)]` module tree
#     (`tests/abi_inline.rs` etc.), not a package. `mfb man tests` errors
#     `unknown package `tests``. It did not exist when this filter was written,
#     so it censused as a phantom 32nd package with PKGDOC `00` — a row that
#     looks like an unfilled overview and is really a directory with no man
#     surface at all. Excluded for the same reason `perf` is.
#
# `general` and `testing` are the reverse case: real registry packages that the
# `mfb man` index deliberately omits (their members are unqualified globals
# needing no IMPORT), but `mfb man general` renders. They ARE in scope.
packages() {
	ls "$BUILTINS" | grep -v '^mod\.rs$' | grep -vE '^(perf|tests)$' | sed 's/^errorcode$/errorCode/' | sort
}

# ---------------------------------------------------------------------------
# The narrative guide topics under src/docs/man/** — `errors`, `flow`, `types`,
# … — embedded at build time by src/docs/man/mod.rs and reached by
# `mfb man <topic>`. plan-108 excluded them entirely, so until plan-125 the man
# census denominator was 31 of the 41 units a developer can actually read, and
# 596 of 628 pages. They are registry-free (plain markdown), but they render
# through the SAME renderer and are held to the same content standard, so every
# sweep here covers them.
#
# Derived from the directory rather than hard-coded: a new topic must not be
# able to appear without the census noticing.
MANDOCS=${MANDOCS:-src/docs/man}
topics() {
	ls "$MANDOCS" | grep -v '^mod\.rs$' | sort
}

# A topic's pages: its overview plus each subtopic file. `mfb man <topic> --all`
# renders all of them, one `═` rule apiece.
topic_pages() {
	local topic=$1
	"$MFB" man "$topic" --all 2>/dev/null | grep -c '^═'
}

# The function names a package's overview page lists. Continuation rows of a
# wrapped summary start with a blank first column, so anchoring on `^│ pkg::`
# counts each function exactly once.
# Members that actually have a page, taken from the overview's *Functions*
# table only. Scoping to that table matters: a package may render other tables
# whose cells are also `pkg::name` — `math`'s Constants table lists `math::pi`
# and friends, which are values with no page at all, and counting them reported
# a phantom "7 pages with no prose".
functions_of() {
	local pkg=$1
	"$MFB" man "$pkg" 2>/dev/null |
		awk '
			/^Functions$/ { infn = 1; next }
			# A bare capitalised word on its own line starts the next section.
			infn && /^[A-Za-z][A-Za-z ]*$/ { infn = 0 }
			infn { print }
		' |
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
# --topics: the narrative-guide half of the denominator.
#
# PAGES is read from the RENDERED output (`═` rules), not from a file count, for
# the same reason every other mode reads rendered output: a markdown file that
# the topic index does not list renders nowhere and is not part of the surface.
# The FILES column is printed beside it precisely so a disagreement between the
# two is visible rather than silently resolved in favour of one of them.
mode_topics() {
	local topics=("$@")
	local t pages files lines fences
	local t_pages=0 t_files=0 t_lines=0 t_fences=0
	printf '%-16s %6s %6s %7s %7s\n' TOPIC PAGES FILES LINES FENCES
	printf -- '------------------------------------------------\n'
	for t in "${topics[@]}"; do
		pages=$(topic_pages "$t")
		files=$(find "$MANDOCS/$t" -name '*.md' | wc -l | tr -d ' ')
		lines=$(find "$MANDOCS/$t" -name '*.md' -exec cat {} + | wc -l | tr -d ' ')
		fences=$(find "$MANDOCS/$t" -name '*.md' -exec cat {} + | grep -c '^```')
		fences=$(( fences / 2 ))
		printf '%-16s %6s %6s %7s %7s\n' "$t" "$pages" "$files" "$lines" "$fences"
		t_pages=$((t_pages+pages)); t_files=$((t_files+files))
		t_lines=$((t_lines+lines)); t_fences=$((t_fences+fences))
		if [ "$pages" -ne "$files" ]; then
			printf '%-16s MISMATCH: %s rendered pages vs %s markdown files\n' "$t" "$pages" "$files"
		fi
	done
	printf -- '------------------------------------------------\n'
	printf '%-16s %6s %6s %7s %7s\n' TOTAL "$t_pages" "$t_files" "$t_lines" "$t_fences"
	[ "$t_pages" -eq "$t_files" ]
}

# ---------------------------------------------------------------------------
# Rendered hits for the banned memory vocabulary, attributed to the page they
# render on.
#
# Carve-out 1 (plan-108-A §3 (2a)): every `borrow` in `datetime` is ARITHMETIC
# borrow ("a negative nanos value borrows a second"), not a memory claim.
#
# Carve-out 2 (plan-108-E): an Errors-table row. Those rows are DERIVED from the
# `errorCode` constant descriptors, whose `message` is the string the runtime
# prints when the error is raised (`_mfb_str_error_allocation` and friends) —
# not authored page prose. `ErrOutOfMemory`'s message is "Allocation failed.",
# so every page that can raise it shows the banned word `allocation` in a cell
# no page author can edit. Changing it would change program output and drift
# goldens, which plan-108 is explicitly barred from doing. The same message is
# derived again into the `errorCode` overview's Constants table (bug-609), a row
# shaped `errorCode::ErrName │ Integer │ code │ message`, carved the same way.
#
# Both are classified and counted separately, never silently dropped.
mode_memory_scope() {
	local pkgs=("$@")
	local pkg fn hits carve=0 carve2=0 carve3=0 carve4=0 unclassified=0

	# Carve-out 3 applies here too: the generated optimizer-catalog rows trip the
	# memory ban ("lifetimes", "frees", "allocation") exactly as they trip the
	# internals ban, and they are just as uneditable. Same region, same rule.
	local cat_lo=0 cat_hi=0
	if [ -z "${SWEEP_TOPICS:-}" ] || printf '%s' "${SWEEP_TOPICS:-}" | grep -q optimizations; then
		cat_lo=$("$MFB" man optimizations --all 2>/dev/null | grep -nx 'Passes' | head -1 | cut -d: -f1)
		cat_hi=$("$MFB" man optimizations --all 2>/dev/null | grep -nxE 'Always-on (lowering|rewrites) \(Level 0\)' | head -1 | cut -d: -f1)
		: "${cat_lo:=0}" "${cat_hi:=0}"
	fi

	scan_page() { # $1 = label, $2 = rendered text
		local label=$1 text=$2 line
		while IFS= read -r line; do
			local n=${line%%:*}
			local body=${line#*:}
			if [ "$pkg" = datetime ] && printf '%s' "$body" | grep -qiE 'borrow'; then
				carve=$((carve + 1))
				printf 'CARVE-1  %-28s %5s  %s\n' "$label" "$n" "$body"
			elif printf '%s' "$body" | grep -qE '^.?.?.?.?[[:space:]]*[0-9]{8}[[:space:]]*.?.?.?.?[[:space:]]*Err[A-Za-z]' ||
				{ [ "$pkg" = errorCode ] && printf '%s' "$body" | grep -qE '^│ errorCode::Err[A-Za-z]+[[:space:]]+│ Integer[[:space:]]+│ [0-9]{8}[[:space:]]+│'; }; then
				carve2=$((carve2 + 1))
				printf 'CARVE-2  %-28s %5s  %s\n' "$label" "$n" "$body"
			elif [ "$label" = 'optimizations (guide)' ] && [ "$cat_hi" -gt 0 ] &&
				[ "$n" -gt "$cat_lo" ] && [ "$n" -lt "$cat_hi" ] &&
				printf '%s' "$body" | grep -q '^│'; then
				carve3=$((carve3 + 1))
				printf 'CARVE-3  %-28s %5s  %s\n' "$label" "$n" "$body"
			# Carve-out 4 (plan-125-B): the `link` guide's C-ABI type rows. A binding
			# author writes C types in an ABI signature, and `CPtr` IS a native
			# pointer; renaming it would make the row false. Only the table rows
			# that define `CString`/`CPtr` are carved — every MFBASIC-facing sentence
			# on the page is held to the ban.
			elif [ "$label" = 'link (guide)' ] &&
				printf '%s' "$body" | grep -qE '^│ *(CString|CPtr) *│'; then
				carve4=$((carve4 + 1))
				printf 'CARVE-4  %-28s %5s  %s\n' "$label" "$n" "$body"
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

	# The guide topics are held to the same ban. `pkg` is reused as the carve-1
	# key and no topic is named `datetime`, so no topic hit can be mis-carved.
	local topic
	for topic in $SWEEP_TOPICS; do
		pkg=$topic
		scan_page "$topic (guide)" "$("$MFB" man "$topic" --all 2>/dev/null)"
	done

	printf '\n'
	printf 'unclassified memory-vocabulary hits: %d\n' "$unclassified"
	printf 'carve-out 1 (datetime arithmetic borrow): %d\n' "$carve"
	printf 'carve-out 2 (derived Errors-table row): %d\n' "$carve2"
	printf 'carve-out 3 (generated optimizer-catalog row): %d\n' "$carve3"
	printf 'carve-out 4 (link C-ABI type row): %d\n' "$carve4"
	[ "$unclassified" -eq 0 ]
}

# ---------------------------------------------------------------------------
# Rendered hits for the compiler-internals vocabulary `.ai/man-content.md` §3
# forbids (SCOPE_CORE). This is what plan-108-F certifies at 0 alongside
# --memory-scope; the two are separate because a page can be clean of memory
# words while still naming a bug number or a mangled symbol.
mode_scope() {
	local pkgs=("$@")
	local pkg fn hits=0 carve3=0

	# Carve-out 3 (plan-125-A Phase 1): the `optimizations` guide's pass table is
	# NOT page prose. `src/cli/man.rs:render_topic_overview` substitutes the
	# `{{optimizer-catalog}}` marker with `optimizer::catalog::render_markdown_table()`
	# at display time, precisely so the page and the compiler can never disagree
	# about which passes exist. Its Stage column is literally `NIR` / `MIR` /
	# `regalloc` / `codegen`, so every row trips the internals sweep, and no page
	# author can edit any of it — the same shape as carve-out 2's derived Errors
	# rows. Counted separately, never silently dropped.
	#
	# The region is bounded by the two rendered headings around the marker, and
	# only box-drawing TABLE ROWS inside it are carved: the authored sentences in
	# the same section (the "Stage says where the pass runs" intro) stay HITs,
	# which is the point — they are prose a reviewer can rewrite.
	local cat_lo=0 cat_hi=0
	if [ -z "${SWEEP_TOPICS:-}" ] || printf '%s' "${SWEEP_TOPICS:-}" | grep -q optimizations; then
		cat_lo=$("$MFB" man optimizations --all 2>/dev/null | grep -nx 'Passes' | head -1 | cut -d: -f1)
		cat_hi=$("$MFB" man optimizations --all 2>/dev/null | grep -nxE 'Always-on (lowering|rewrites) \(Level 0\)' | head -1 | cut -d: -f1)
		: "${cat_lo:=0}" "${cat_hi:=0}"
	fi

	scope_page() { # $1 = label, $2 = rendered text
		local label=$1 text=$2 line
		while IFS= read -r line; do
			local n=${line%%:*}
			local body=${line#*:}
			if [ "$label" = 'optimizations (guide)' ] && [ "$cat_hi" -gt 0 ] &&
				[ "$n" -gt "$cat_lo" ] && [ "$n" -lt "$cat_hi" ] &&
				printf '%s' "$body" | grep -q '^│'; then
				carve3=$((carve3 + 1))
				printf 'CARVE-3  %-28s %5s  %s\n' "$label" "$n" "$body"
				continue
			fi
			hits=$((hits + 1))
			printf 'HIT      %-28s %5s  %s\n' "$label" "$n" "$body"
		# Case-SENSITIVE on purpose: several patterns are all-caps host names
		# (`EINTR`, `EEXIST`) whose lowercase forms are ordinary English, and
		# `errno` case-folded is a prefix of `ErrNotFound`.
		done < <(printf '%s\n' "$text" | grep -nE "$(scope_regex)")
	}

	for pkg in "${pkgs[@]}"; do
		scope_page "$pkg (overview)" "$("$MFB" man "$pkg" 2>/dev/null)"
		if has_types_page "$pkg"; then
			scope_page "$pkg (types)" "$("$MFB" man "$pkg" types 2>/dev/null)"
		fi
		for fn in $(functions_of "$pkg"); do
			scope_page "$pkg::$fn" "$("$MFB" man "$pkg" "$fn" 2>/dev/null)"
		done
	done

	local topic
	for topic in $SWEEP_TOPICS; do
		scope_page "$topic (guide)" "$("$MFB" man "$topic" --all 2>/dev/null)"
	done

	printf '\n'
	printf 'internals-vocabulary hits: %d\n' "$hits"
	printf 'carve-out 3 (generated optimizer-catalog row): %d\n' "$carve3"
	[ "$hits" -eq 0 ]
}

# ---------------------------------------------------------------------------
main() {
	local mode=fill
	case "${1:-}" in
	--fill) mode=fill; shift ;;
	--functions) mode=functions; shift ;;
	--memory-scope) mode=memory-scope; shift ;;
	--scope) mode=scope; shift ;;
	--topics) mode=topics; shift ;;
	--banned-list) printf '%s\n' "$BANNED_CORE"; return 0 ;;
	-h | --help) usage; return 0 ;;
	esac

	if [ ! -x "$MFB" ]; then
		echo "man-census.sh: no mfb binary at $MFB (cargo build --release)" >&2
		return 2
	fi

	if [ "$mode" = topics ]; then
		local tps
		if [ "$#" -gt 0 ]; then
			tps=("$@")
		else
			# shellcheck disable=SC2207
			tps=($(topics))
		fi
		mode_topics "${tps[@]}"
		return
	fi

	local pkgs
	# SWEEP_TOPICS is the guide-topic half of the two vocabulary sweeps. A run
	# scoped to named packages is answering a question about those packages, so
	# it stays scoped; only a whole-surface run (no arguments) sweeps the topics
	# as well — otherwise `--scope color` would silently re-report every topic.
	if [ "$#" -gt 0 ]; then
		pkgs=("$@")
		SWEEP_TOPICS=''
	else
		# shellcheck disable=SC2207
		pkgs=($(packages))
		SWEEP_TOPICS=$(topics | tr '\n' ' ')
	fi

	case "$mode" in
	fill) mode_fill "${pkgs[@]}" ;;
	functions) mode_functions "${pkgs[@]}" ;;
	memory-scope) mode_memory_scope "${pkgs[@]}" ;;
	scope) mode_scope "${pkgs[@]}" ;;
	esac
}

main "$@"

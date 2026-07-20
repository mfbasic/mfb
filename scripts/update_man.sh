#!/usr/bin/env bash
# For each compiler built-in function, uses the claude CLI to review and update/create its man page.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$REPO_ROOT"

# Optional first argument: restrict to a single package/module (e.g. `term`).
# When given, only functions whose module matches are processed.
FILTER="${1:-}"

# Full man-page template, modeled on Linux man(1) pages but detailing each
# built-in function. Placeholders in <angle brackets> are filled in per function;
# sections in [brackets] are conditional and omitted when they do not apply.
# Loaded from .ai/man_template.md.
TEMPLATE="$(cat "$REPO_ROOT/.ai/man_template.md")"

# Template for the per-package consolidated TYPE page (mfb man <pkg> types).
# It documents every built-in record type a package exports, on one page.
# Loaded from .ai/man_type_template.md.
TYPE_TEMPLATE="$(cat "$REPO_ROOT/.ai/man_type_template.md")"

# Authoring rules shared with update_man_package.sh (bug-336-S11): the renderer
# subset, the provenance rule, the Errors-table block, and the correctness rule.
# shellcheck source=man_rules.sh
source "$SCRIPT_DIR/man_rules.sh"

# The package a function belongs to, from its listed name.
#
# A qualified name carries it (`math::sin` -> `math`). An unqualified one is a
# bare always-in-scope builtin and belongs to `general` — EXCEPT the `testing`
# assertions, which are also called bare (`expectEqual(a, b)` inside a `TCASE`)
# but live in their own package and their own man directory. Without this arm,
# `update_man.sh testing` routes all twelve to `general`, matches nothing, and
# exits 1 — the same class of invisibility bug-336-S2 fixed in
# `list_functions.py`.
module_of() {
  case "$1" in
    *::*) printf '%s' "${1%%::*}" ;;
    expect*) printf 'testing' ;;
    *) printf 'general' ;;
  esac
}

# Parse list_functions.py into "kind|name" rows. Lines look like:
#   "  http::read  (function)" / "  math::PI  (constant)" / "  http::Response  (type)"
# Functions and constants get one page each; types are grouped per package into a
# single consolidated `types` page.
FUNCTIONS=()
TYPE_PKGS=()
while IFS= read -r row; do
  kind="${row%%|*}"
  name="${row#*|}"
  module="$(module_of "$name")"

  if [[ -n "$FILTER" && "$module" != "$FILTER" ]]; then
    continue
  fi

  if [[ "$kind" == "type" ]]; then
    # Record the module once; the per-package page covers all its types.
    found=0
    for m in "${TYPE_PKGS[@]:-}"; do
      [[ "$m" == "$module" ]] && { found=1; break; }
    done
    [[ $found -eq 0 ]] && TYPE_PKGS+=("$module")
  else
    FUNCTIONS+=("$name")
  fi
done < <(
  python3 "$SCRIPT_DIR/list_functions.py" \
    | sed -nE 's/^[[:space:]]*(.*)  \((function|constant|type)\)$/\2|\1/p'
)

if [[ -n "$FILTER" && ${#FUNCTIONS[@]} -eq 0 && ${#TYPE_PKGS[@]} -eq 0 ]]; then
  echo "No built-in functions or types found for package '$FILTER'." >&2
  exit 1
fi

total=$(( ${#FUNCTIONS[@]} + ${#TYPE_PKGS[@]} ))
echo "Updating man pages: ${#FUNCTIONS[@]} functions, ${#TYPE_PKGS[@]} type pages..."
echo ""

for i in "${!FUNCTIONS[@]}"; do
  func="${FUNCTIONS[$i]}"
  count=$((i + 1))

  # Derive the source module name for compiler code lookup
  if [[ "$func" == *::* ]]; then
    module="${func%%::*}"
  else
    module="general"
  fi

  # Local function name after the :: (or the full name if no ::)
  fname="${func##*::}"

  echo "=== [$count/$total] $func ==="

  claude -p --dangerously-skip-permissions "Update the mfb man page for the built-in function '$func'.

Steps:
1. Man pages are Markdown, rendered to the terminal by src/docs/render.rs. Read a
   few existing pages (e.g. src/docs/man/unicode/package.md, plus any
   src/docs/man/builtins/**/*.md) for tone and house style, and follow the
   Markdown template below for structure.
2. Read the package's compiler source — $(man_package_sources "${module}") — to
   understand the function's signature, overloads, parameter types, return type,
   and error behavior. When a package has BOTH a .rs and a _package.mfb, the .mfb
   is where the real signatures, defaults, and error paths live; the .rs carries
   the name constants, the is_*_call predicate, return-type tables, and
   call_param_names. Read both; documenting the .rs shim alone gets the
   signatures wrong.
3. Determine every error the function can itself raise. Read
   src/target/shared/code/mod.rs for the canonical error registry: each ERR_*_CODE
   constant maps a symbolic name (e.g. ErrInvalidArgument) to its numeric code
   (e.g. 77050002). Match each failure path in the function to its code and name.
4. Find the existing man page for '${fname}' by looking in
   src/docs/man/builtins/*/${fname}.{md,txt}, or determine the correct path to
   create a new one following the existing directory layout.
   (Collection helpers are namespaced under collections/; the String overloads of
   find/mid/replace live under strings/; only the universal core — len, error,
   conversions, typeName, numeric/empty predicates — lives under general/ or filters/.
   Check which subdirectory best matches existing peers.)
5. Write the page as Markdown to '<dir>/${fname}.md', creating the directory if
   needed. If a legacy plain-text '<dir>/${fname}.txt' exists, delete it (git rm)
   so the package does not end up with a duplicate page.

Format rules:
- The page is Markdown. The '# <localName>' title is the local name (no module
  prefix); the line right after it is the one-line summary.
- SYNOPSIS goes in a fenced code block and uses :: for the module separator
  (e.g. fs::readText, math::abs), one signature line per overload. Unnamespaced
  general functions have no module prefix in the synopsis.
- Section headings (## ...) in order: Synopsis, [Package], [Imports], Description,
  [Overloads if multiple signatures], Parameters, Return value, Errors,
  [Type checking if generic], Examples, [See also].
- Parameters, Return value, and Errors are GFM pipe tables (see the template).
- Follow the full template below exactly. Bracketed sections are conditional;
  omit them when they do not apply. All other sections are required.
\$MAN_RULE_RENDERER
\$MAN_RULE_PROVENANCE

\$MAN_RULE_ERRORS

\$MAN_RULE_CORRECTNESS

$TEMPLATE"

  echo ""
done

# One consolidated types page per package that exports record types.
for j in "${!TYPE_PKGS[@]:-}"; do
  [[ -z "${TYPE_PKGS[*]:-}" ]] && break
  module="${TYPE_PKGS[$j]}"
  count=$(( ${#FUNCTIONS[@]} + j + 1 ))

  echo "=== [$count/$total] $module::types ==="

  claude -p --dangerously-skip-permissions "Update the mfb man page that documents the built-in record types of the '$module' package, as a single consolidated page reached via 'mfb man $module types'.

Steps:
1. Man pages are Markdown, rendered to the terminal by src/docs/render.rs. Read a
   few existing pages (e.g. src/docs/man/unicode/package.md, plus any
   src/docs/man/builtins/**/*.md and src/docs/man/types/*) for tone and house
   style, and follow the Markdown template below for structure.
2. Read the package source src/builtins/${module}_package.mfb. Find every
   'EXPORT TYPE <Name> ... END TYPE' block. For each one, capture its fields:
   the 'field AS Type' lines and the trailing ' comment that explains each field.
3. Write the page as Markdown to src/docs/man/builtins/${module}/types.md (create
   the directory if needed). The file stem MUST be exactly 'types' so
   'mfb man $module types' resolves it; do NOT create one file per type. If a
   legacy 'types.txt' exists, delete it (git rm) so there is no duplicate page.

Format rules:
- The page is Markdown. The '# types' title is followed by the one-line summary.
- Section headings (## ...) in order: Synopsis, Package, [Imports], Description,
  Types, [See also]. Imports and See also are optional; omit them when they do
  not apply. All other sections are required.
- The Synopsis fenced block lists each exported type, one '${module}::<TypeName>'
  per line.
- Document EVERY exported type on this one page, under the single '## Types'
  section, in source order. Give each type its own '### ${module}::<TypeName>'
  subheading, a one-line description, then a GFM pipe table of its fields
  (Field, Type, Description).
- Derive each field's description from the source comment (units, ranges,
  defaults, what each value selects).
- Follow the full template below exactly.
\$MAN_RULE_RENDERER
\$MAN_RULE_PROVENANCE

$TYPE_TEMPLATE"

  echo ""
done

echo "Done. Updated $total man pages ($((${#FUNCTIONS[@]})) functions, ${#TYPE_PKGS[@]} type pages)."

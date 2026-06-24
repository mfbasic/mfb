#!/usr/bin/env bash
# For each built-in package, uses the claude CLI to review and update/create its
# package.txt man page (the per-module overview, not the per-function pages).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$REPO_ROOT"

# Full package-page template, modeled on the SYNOPSIS/DESCRIPTION layout of Linux
# man(1) overview pages but describing a whole built-in package. Placeholders in
# <angle brackets> are filled in per package; sections in [brackets] are
# conditional and omitted when they do not apply.
read -r -d '' TEMPLATE <<'TEMPLATE_EOF' || true
Full package template (fill in every <...> placeholder; keep section names and order):

NAME
  <package> - <one-line description of what the package is for>

SYNOPSIS
  IMPORT <package>
  <package>::<function>(<args>)
  [a few representative calls only -- NOT the full function list]

[IMPORTS]
  <ONLY for always-available packages (general, filters): state that the
  functions are in scope without any IMPORT. Omit this section entirely for
  importable packages -- the IMPORT line in SYNOPSIS already covers them.>

DESCRIPTION
  <Package-level overview: the purpose of the package, the kinds of values and
  built-in types it works with, the conventions shared across its functions
  (indexing, units, ownership, mutation, ordering), and any cross-cutting
  behavior. Define any built-in types the package introduces. Do not repeat
  every per-function detail; describe what is common to the whole package.>

ERRORS
  <code> (<ErrName>)
  <the package-wide condition under which the package's functions raise this error>

  <code> (<ErrName>)
  <the package-wide condition under which the package's functions raise this error>

The page ends after ERRORS. Do NOT add EXAMPLES, SEE ALSO, or a list of the
package's functions: the `mfb man <package>` command renders the package.txt and
then automatically appends the full FUNCTIONS list and a "Run `mfb man <package>
<function>`" footer, so any such content in package.txt would be duplicated.
TEMPLATE_EOF

# Collect the documented built-in packages (each has a package.txt overview).
PACKAGES=()
while IFS= read -r dir; do
  PACKAGES+=("$(basename "$dir")")
done < <(find src/man/builtins -mindepth 1 -maxdepth 1 -type d | sort)

total=${#PACKAGES[@]}
echo "Updating package man pages for $total packages..."
echo ""

for i in "${!PACKAGES[@]}"; do
  pkg="${PACKAGES[$i]}"
  count=$((i + 1))

  # Locate the package's compiler source. Most packages map to src/builtins/<pkg>.rs;
  # some also have an .mfb package file, and a few are special-cased.
  sources=()
  case "$pkg" in
    filters)
      # The predicate helpers (isEven, isEmpty, ...) live in general.rs.
      sources+=("src/builtins/general.rs")
      ;;
    *)
      [[ -f "src/builtins/${pkg}.rs" ]] && sources+=("src/builtins/${pkg}.rs")
      [[ -f "src/builtins/${pkg}_package.mfb" ]] && sources+=("src/builtins/${pkg}_package.mfb")
      ;;
  esac
  if [[ ${#sources[@]} -eq 0 ]]; then
    src_list="(no dedicated source file; infer the package from its function man pages)"
  else
    src_list="$(printf '%s, ' "${sources[@]}")"
    src_list="${src_list%, }"
  fi

  echo "=== [$count/$total] $pkg ==="

  claude -p --dangerously-skip-permissions "Update the mfb package man page for the built-in package '$pkg'.
This is the package overview page at src/man/builtins/${pkg}/package.txt, not a per-function page.

Steps:
1. Read several src/man/builtins/*/package.txt files to understand the package-page
   format and style conventions (how SYNOPSIS, DESCRIPTION, and SEE ALSO are written
   for a whole package rather than a single function).
2. Read the package's compiler source (${src_list}) to understand which functions and
   constants it exports, the built-in types it defines, and its shared conventions.
3. Read the per-function man pages in src/man/builtins/${pkg}/*.txt (every .txt except
   package.txt) so the DESCRIPTION reflects the conventions those pages share. Do NOT
   list every function in package.txt -- the 'mfb man ${pkg}' command appends that list
   automatically (see the note below).
4. Determine the errors the package's functions can raise. Read src/man/errors/package.txt
   for the Error model and src/target/shared/code/mod.rs for the canonical error registry:
   each ERR_*_CODE constant maps a symbolic name (e.g. ErrInvalidArgument) to its numeric
   code (e.g. 77050002). Collect the codes that functions in this package raise.
5. Write the updated package.txt at src/man/builtins/${pkg}/package.txt.

Format rules:
- NAME line: '  <package> - <one-line description>'
- SYNOPSIS opens with the IMPORT line (or, for always-available packages such as
  general and filters, note that no IMPORT is needed) followed by only a few
  representative calls that illustrate typical usage, using :: for the module
  separator. Do NOT list every function -- 'mfb man ${pkg}' appends the full list.
- Standard sections in order: NAME, SYNOPSIS, [IMPORTS], DESCRIPTION, ERRORS
- Include IMPORTS only for always-available packages (general, filters) to note
  that no IMPORT is needed; omit it for importable packages.
- The page ends after ERRORS. Do not add EXAMPLES, SEE ALSO, or a FUNCTIONS list;
  'mfb man ${pkg}' appends the function list and footer automatically.
- Follow the full template below exactly. Sections in [brackets] are conditional;
  omit them when they do not apply. All other sections are required.
- Two-space indent for all content within sections
- Blank line between sections

ERRORS section (required, always present):
- This is the package-wide summary of error behavior. List each distinct error that
  functions in this package can raise. For each error write the numeric code, then
  the symbolic name in parentheses, on one line; put the description on the following
  line(s):

    ERRORS
      77050002 (ErrInvalidArgument)
      Raised by <functions> when <condition>.

      77050010 (ErrOverflow)
      Raised by <functions> when <condition>.

- Use the exact code<->name pairs from src/target/shared/code/mod.rs. Do not invent
  codes or names.
- If no function in this package raises any error, the section must read exactly:

    ERRORS
      No errors.

  (Errors propagating from evaluating arguments before a call do not count; do not
  list them.)

$TEMPLATE"

  echo ""
done

echo "Done. Updated $total package man pages."

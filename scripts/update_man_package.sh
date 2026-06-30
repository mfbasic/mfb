#!/usr/bin/env bash
# For each built-in package, uses the claude CLI to review and update/create its
# package.md man page (the per-module overview, not the per-function pages).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$REPO_ROOT"

# Markdown template for the per-package overview page (mfb man <package>),
# describing a whole built-in package rather than a single function.
# Loaded from .ai/man_package_template.md.
TEMPLATE="$(cat "$REPO_ROOT/.ai/man_package_template.md")"

# Collect the documented built-in packages (each has a package.{txt,md} overview).
PACKAGES=()
while IFS= read -r dir; do
  PACKAGES+=("$(basename "$dir")")
done < <(find src/docs/man/builtins -mindepth 1 -maxdepth 1 -type d | sort)

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
This is the package overview page at src/docs/man/builtins/${pkg}/package.md, not a per-function page.

Steps:
1. Man pages are Markdown, rendered to the terminal by src/docs/render.rs. Read a
   few existing overview pages (e.g. src/docs/man/unicode/package.md, plus any
   src/docs/man/builtins/*/package.md) for tone and house style, and follow the
   Markdown template below for structure.
2. Read the package's compiler source (${src_list}) to understand which functions and
   constants it exports, the built-in types it defines, and its shared conventions.
3. Read the per-function man pages in src/docs/man/builtins/${pkg}/* (every page except
   the package overview) so the Description reflects the conventions those pages share.
   Do NOT list every function -- the 'mfb man ${pkg}' command appends that list
   automatically (see the note below).
4. Determine the errors the package's functions can raise. Read src/docs/man/errors/package.txt
   for the Error model and src/target/shared/code/mod.rs for the canonical error registry:
   each ERR_*_CODE constant maps a symbolic name (e.g. ErrInvalidArgument) to its numeric
   code (e.g. 77050002). Collect the codes that functions in this package raise.
5. Write the page as Markdown to src/docs/man/builtins/${pkg}/package.md. If a legacy
   plain-text package.txt exists, delete it (git rm) so there is no duplicate overview.

Format rules:
- The page is Markdown. The '# ${pkg}' title is followed by the one-line summary.
- Section headings (## ...) in order: Synopsis, [Imports], Description, Errors.
- The Synopsis fenced block opens with the IMPORT line (or, for always-available
  packages such as general and filters, a note that no IMPORT is needed) followed
  by only a few representative calls that illustrate typical usage, using :: for
  the module separator. Do NOT list every function -- 'mfb man ${pkg}' appends it.
- Include Imports only for always-available packages (general, filters) to note
  that no IMPORT is needed; omit it for importable packages.
- The page ends after Errors. Do not add Examples, See also, or a function list;
  'mfb man ${pkg}' appends the function list and footer automatically.
- Follow the full template below exactly. Bracketed sections are conditional;
  omit them when they do not apply. All other sections are required.
- Use the renderer's supported Markdown subset only: ATX headings, paragraphs,
  bullet/ordered lists, fenced code blocks, pipe tables, and inline
  \`code\`/**bold**/*italic*/[links](url).
- Provenance: back a non-obvious implementation claim (error code, shared
  convention, magic number, offset, enum variant) with an invisible
  \`[[src/file.rs:Symbol]]\` citation at claim-cluster granularity — symbol-preferred,
  \`[[src/file.rs:line]]\` only where no symbol fits. Grep-confirm the symbol exists
  before citing. The renderer strips \`[[ ]]\` everywhere (including headings), so
  they never display in 'mfb man' output but keep claims traceable for reviewers.
  Do not add non-verifiable claims.

Errors table (required, always present):
- This is the package-wide summary of error behavior. List each distinct error
  functions in this package can raise, one row per error, with the numeric code,
  the symbolic name, and the condition:

    ## Errors

    | Code | Name | Raised when |
    | --- | --- | --- |
    | \`77050002\` | \`ErrInvalidArgument\` | raised by <functions> when <condition> |
    | \`77050010\` | \`ErrOverflow\` | raised by <functions> when <condition> |

- Use the exact code<->name pairs from src/target/shared/code/mod.rs. Do not invent
  codes or names.
- If no function in this package raises any error, replace the table with a single
  line that reads exactly: No errors.
  (Errors propagating from evaluating arguments before a call do not count; do not
  list them.)

$TEMPLATE"

  echo ""
done

echo "Done. Updated $total package man pages."

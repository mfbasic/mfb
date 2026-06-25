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
read -r -d '' TEMPLATE <<'TEMPLATE_EOF' || true
Full template (fill in every <...> placeholder; keep section names and order):

NAME
  <localName> - <one-line description of what the function does>

SYNOPSIS
  <module>::<localName>(<param> AS <Type>, ...) AS <ReturnType>
  [one extra line per additional overload signature]

[PACKAGE]
  <name of the package this function belongs to>

[IMPORTS]
  <the IMPORT statement a program needs, or a note that it is always available>

DESCRIPTION
  <Thorough prose: what the function computes, how each argument is interpreted,
  boundary and edge-case behavior, units, ordering, mutation/side effects, and
  any platform notes. This is the heart of the page; be specific and complete.>

[OVERLOADS]
  <signature>
    <what this particular overload does and how it differs from the others>

  <signature>
    <what this particular overload does and how it differs from the others>

PARAMETERS
  <param> AS <Type>
    <meaning, accepted range/values, units, and what each value selects>

  <param> AS <Type>
    <meaning, accepted range/values, units, and what each value selects>

RETURN VALUE
  AS <ReturnType>
    <what is returned on success, including boundary results and special cases>

ERRORS
  <code> (<ErrName>)
  <the condition under which this error is raised>

  <code> (<ErrName>)
  <the condition under which this error is raised>

[TYPE CHECKING]
  <the types this generic function accepts and any compile-time constraints>

EXAMPLES
  <short caption describing the example>:

    <runnable mfb code>

[SEE ALSO]
  <relatedFunction>, <relatedFunction>
TEMPLATE_EOF

# Collect all function/constant names, strip leading whitespace and the "(constant)" label
FUNCTIONS=()
while IFS= read -r line; do
  FUNCTIONS+=("$line")
done < <(
  python3 "$SCRIPT_DIR/list_functions.py" \
    | grep '^\s' \
    | sed 's/^[[:space:]]*//' \
    | sed 's/[[:space:]]*(constant)//'
)

# If a package filter was given, keep only functions in that module.
if [[ -n "$FILTER" ]]; then
  KEPT=()
  for func in "${FUNCTIONS[@]}"; do
    if [[ "$func" == *::* ]]; then
      module="${func%%::*}"
    else
      module="general"
    fi
    if [[ "$module" == "$FILTER" ]]; then
      KEPT+=("$func")
    fi
  done
  if [[ ${#KEPT[@]} -eq 0 ]]; then
    echo "No built-in functions found for package '$FILTER'." >&2
    exit 1
  fi
  FUNCTIONS=("${KEPT[@]}")
fi

total=${#FUNCTIONS[@]}
echo "Updating man pages for $total functions..."
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
1. Read src/man/builtins/**/*.txt to understand the man page format and style conventions.
2. Read src/builtins/${module}.rs to understand the function's signature, overloads,
   parameter types, return type, and error behavior.
3. Determine every error the function can itself raise. Read
   src/target/shared/code/mod.rs for the canonical error registry: each ERR_*_CODE
   constant maps a symbolic name (e.g. ErrInvalidArgument) to its numeric code
   (e.g. 77050002). Match each failure path in the function to its code and name.
4. Find the existing man page for '${fname}' by looking in src/man/builtins/*/${fname}.txt,
   or determine the correct path to create a new one following the existing directory layout.
   (Collection helpers are namespaced under collections/; the String overloads of
   find/mid/replace live under strings/; only the universal core — len, error,
   conversions, typeName, numeric/empty predicates — lives under general/ or filters/.
   Check which subdirectory best matches existing peers.)
5. Write the updated or new .txt file at that path, creating the directory if needed.

Format rules:
- NAME line: '  <localName> - <one-line description>'
- SYNOPSIS uses :: for the module separator (e.g. fs::readText, math::abs)
  Unnamespaced general functions have no module prefix in SYNOPSIS
- Standard sections in order: NAME, SYNOPSIS, [PACKAGE], [IMPORTS], DESCRIPTION,
  [OVERLOADS if multiple signatures], PARAMETERS, RETURN VALUE, ERRORS,
  [TYPE CHECKING if generic], EXAMPLES, [SEE ALSO]
- Follow the full template below exactly. Sections in [brackets] are conditional;
  omit them when they do not apply. All other sections are required.
- Two-space indent for all content within sections
- Blank line between sections

ERRORS section (required, always present):
- List every error the function can itself raise. For each error write the
  numeric code, then the symbolic name in parentheses, on one line; put the
  description on the following line(s):

    ERRORS
      77050002 (ErrInvalidArgument)
      Raised when <condition>.

      77050001 (ErrIndexOutOfRange)
      Raised when <condition>.

- Use the exact code<->name pairs from src/target/shared/code/mod.rs. Do not
  invent codes or names.
- If the function cannot itself raise any error, the section must read exactly:

    ERRORS
      No errors.

  (Errors propagating from evaluating arguments before the call do not count as
  errors this function raises; do not list them.)

$TEMPLATE"

  echo ""
done

echo "Done. Updated $total man pages."

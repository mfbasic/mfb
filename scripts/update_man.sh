#!/usr/bin/env bash
# For each compiler built-in function, uses the claude CLI to review and update/create its man page.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$REPO_ROOT"

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
3. Find the existing man page for '${fname}' by looking in src/man/builtins/*/${fname}.txt,
   or determine the correct path to create a new one following the existing directory layout.
   (Collection helpers are namespaced under collections/; the String overloads of
   find/mid/replace live under strings/; only the universal core — len, error,
   conversions, typeName, numeric/empty predicates — lives under general/ or filters/.
   Check which subdirectory best matches existing peers.)
4. Write the updated or new .txt file at that path, creating the directory if needed.

Format rules:
- NAME line: '  <localName> - <one-line description>'
- SYNOPSIS uses :: for the module separator (e.g. fs::readText, math::abs)
  Unnamespaced general functions have no module prefix in SYNOPSIS
- Standard sections in order: NAME, SYNOPSIS, [PACKAGE], [IMPORTS], DESCRIPTION,
  [OVERLOADS if multiple signatures], [ERRORS], [TYPE CHECKING if generic], EXAMPLES,
  [SEE ALSO]
- Two-space indent for all content within sections
- Blank line between sections"

  echo ""
done

echo "Done. Updated $total man pages."

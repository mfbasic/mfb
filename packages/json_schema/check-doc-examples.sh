#!/usr/bin/env bash
#
# check-doc-examples.sh — compile and run every EXAMPLE in the package's DOC
# blocks.
#
# A DOC block's prose fields are strings the compiler never reads, so nothing
# else here can tell you that an example stopped compiling — or never did. This
# script is that check: it extracts each `EXAMPLE ... END EXAMPLE` from
# `src/lib.mfb`, builds it as a project against this package, and runs it.
#
#     ./check-doc-examples.sh [path/to/mfb]
#
# An example that declares only a helper function gets a trivial `main` appended,
# so a fragment showing a TRAP handler is still compiled rather than skipped.
# Exit status is 0 iff every example built and ran.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
MFB="${1:-$ROOT/target/release/mfb}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [[ ! -x "$MFB" ]]; then
  echo "no compiler at $MFB — pass one as \$1, or run: cargo build --release --bin mfb" >&2
  exit 1
fi

python3 - "$HERE" "$WORK" <<'PY'
import os, re, sys

package, work = sys.argv[1], sys.argv[2]
source = open(os.path.join(package, "src", "lib.mfb"), encoding="utf-8").read()
blocks = re.findall(r"^  EXAMPLE\n(.*?)^  END EXAMPLE\n", source, flags=re.S | re.M)

for index, block in enumerate(blocks):
    body = "\n".join(
        line[4:] if line.startswith("    ") else line for line in block.split("\n")
    )
    if not re.search(r"^(FUNC|SUB) main", body, flags=re.M):
        body += "\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n"
    project = os.path.join(work, f"ex{index}")
    os.makedirs(os.path.join(project, "src"), exist_ok=True)
    with open(os.path.join(project, "project.json"), "w", encoding="utf-8") as manifest:
        manifest.write(
            '{"name":"ex%d","version":"0.1.0","mfb":"1.0","kind":"executable",'
            '"description":"a DOC example",'
            '"sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],'
            '"packages":[{"name":"json_schema","version":"=0.1.0","source":"local://%s"}],'
            '"entry":"main","targets":["native"]}\n' % (index, package)
        )
    with open(os.path.join(project, "src", "main.mfb"), "w", encoding="utf-8") as out:
        out.write(body)
print(len(blocks))
PY

count=$(ls -d "$WORK"/ex* 2>/dev/null | wc -l | tr -d ' ')
failures=0
for project in "$WORK"/ex*; do
  name="$(basename "$project")"
  if ! log="$("$MFB" build -q "$project" 2>&1)"; then
    echo "FAIL $name did not build"
    echo "$log" | sed 's/^/     /'
    failures=$((failures + 1))
    continue
  fi
  binary="$project/build/$name.out"
  [[ -x "$binary" ]] || binary="$project/build/$name.exe"
  if ! output="$("$binary" 2>&1)"; then
    echo "FAIL $name did not run"
    echo "$output" | sed 's/^/     /'
    failures=$((failures + 1))
    continue
  fi
  echo "ok   $name"
  [[ -n "$output" ]] && echo "$output" | sed 's/^/       /'
done

echo
if [[ "$failures" -gt 0 ]]; then
  echo "$failures of $count example(s) failed"
  exit 1
fi
echo "all $count example(s) built and ran"

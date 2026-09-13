#!/usr/bin/env bash
#
# smoke.sh — build the package, then build and run a real importer against the
# compiled .mfp.
#
# `mfb test packages/cli` exercises the package from inside, where the parser
# can be handed a literal argument list. This script is the other half: it
# proves the EXPORTED surface works through a package boundary, and that the
# parser reads a genuine process command line from os::args().
#
#     ./smoke.sh [path/to/mfb]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MFB="${1:-$ROOT/target/release/mfb}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# `set -e` does not abort on a failing `[[ ]]` under the bash 3.2 that macOS
# ships, so every assertion says so itself. Never write a bare `[[ ]]` here.
fail() { echo "FAIL: $*" >&2; exit 1; }

"$MFB" build -q "$ROOT/packages/cli"
mkdir -p "$WORK/src" "$WORK/packages"
cp "$ROOT/packages/cli/cli.mfp" "$WORK/packages/cli.mfp"
printf '%s\n' '{"name":"cli_smoke","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","packages":[{"name":"cli","version":"=0.1.0","source":"file:packages/cli.mfp"}]}' > "$WORK/project.json"
cat > "$WORK/src/main.mfb" <<'MFB'
IMPORT cli
IMPORT io

FUNC main() AS Integer
  LET options AS List OF cli::Option = [cli::Option[name := "port", alias := "p", required := TRUE, kind := cli::OptionKind.Integer], cli::Option[name := "offset", alias := "o", required := FALSE, kind := cli::OptionKind.Integer], cli::Option[name := "enabled", alias := "b", required := FALSE, kind := cli::OptionKind.Bool], cli::Option[name := "title", alias := "t", required := FALSE, kind := cli::OptionKind.String], cli::Option[name := "quiet", alias := "", required := FALSE, kind := cli::OptionKind.Flag]]
  cli::parse(options)
  io::print(toString(cli::getInteger("port", 0)))
  io::print(toString(cli::getInteger("offset", 99)))
  io::print(toString(cli::getBool("enabled", FALSE)))
  io::print("title=[" & cli::getString("title", "FALLBACK") & "]")
  io::print("quiet=" & toString(cli::getFlag("quiet", FALSE)))
  FOR EACH operand IN cli::operands()
    io::print("operand=" & operand)
  NEXT
  cli::showUsage(["header"], options, ["footer"])
  RETURN 0
END FUNC
MFB
"$MFB" build -q "$WORK"
BIN="$WORK/build/cli_smoke.out"

# Assert that running BIN with the given arguments prints exactly $1.
# `sed -n Np` pulls one line out so a check names the value it is about.
expect() {
  local want="$1"; shift
  local got
  got="$("$BIN" "$@" 2>&1)" || fail "mfb exited non-zero for: $*"
  [[ "$got" == "$want" ]] || fail "for [$*]
  expected: $want
       got: $got"
}
expect_line() {
  local n="$1" want="$2"; shift 2
  local got
  got="$("$BIN" "$@" 2>&1 | sed -n "${n}p")" || fail "mfb exited non-zero for: $*"
  [[ "$got" == "$want" ]] || fail "for [$*] line $n
  expected: $want
       got: $got"
}
refuses() {
  set +e
  "$BIN" "$@" >/dev/null 2>&1
  local status=$?
  set -e
  [[ "$status" -ne 0 ]] || fail "expected a non-zero exit for: $*"
}

# The usage table renders every kind, both alias columns, and `(required)`.
usage="$("$BIN" -p 3000 -b true)"
for row in \
  '3000' '99' 'TRUE' 'title=[FALLBACK]' 'quiet=FALSE' 'header' \
  '  -p, --port <integer> (required)' \
  '  -o, --offset <integer>' \
  '  -b, --enabled <bool>' \
  '  -t, --title <string>' \
  '      --quiet' \
  'footer'
do
  [[ "$usage" == *"$row"* ]] || fail "usage output is missing the line: $row
$usage"
done
[[ "$usage" == *"Usage: $BIN [options]"* ]] || fail "usage line does not name the program:
$usage"

# Short and long spellings, and every accepted boolean literal.
expect_line 3 'TRUE'  -p 3000 -b true
expect_line 3 'TRUE'  -p 3000 -b 1
expect_line 3 'FALSE' -p 3000 -b false
expect_line 3 'FALSE' -p 3000 -b 0
expect_line 1 '3000'  --port 3000 -b 1
expect_line 1 '3000'  --port=3000 -b 1

# An inline value is the only way to pass one that begins with a dash.
expect_line 2 '-3'          -p 1 --offset=-3
expect_line 4 'title=[a=b]' -p 1 --title=a=b

# A supplied empty value is not the fallback.
expect_line 4 'title=[]' -p 1 --title=
expect_line 4 'title=[]' -p 1 -t ''

# A flag is present-or-absent.
expect_line 5 'quiet=FALSE' -p 1
expect_line 5 'quiet=TRUE'  -p 1 --quiet

# `--` hands the rest through untouched, option-looking or not.
operands="$("$BIN" -p 1 -- report.txt -p --nope)"
[[ "$operands" == *$'operand=report.txt\noperand=-p\noperand=--nope'* ]] \
  || fail "operands after -- were not passed through verbatim:
$operands"

# Everything the parser refuses must exit non-zero.
refuses -p                 # a non-flag option with no value
refuses -p 1 --offset -3   # a separate value beginning with a dash
refuses                    # a required option left out
refuses -p 1 --nope        # an unknown option
refuses -p 1 -p 2          # a duplicate
refuses -p 1 --quiet=true  # a flag given a value
refuses -p=1               # a short option with an inline value
refuses -p 1 -b maybe      # a boolean that is not one of the four spellings
refuses -p 1 -o twelve     # an integer that is not a number

echo "cli smoke passed"

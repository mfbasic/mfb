#!/usr/bin/env bash
#
# smoke.sh — build the package, then build and run a real importer against the
# compiled .mfp.
#
# `mfb test packages/cli` exercises the parser through `tryParse` with literal
# argument lists. This script is the other half: it proves the EXPORTED surface
# works through a package boundary, that `cli::parse` reads the genuine process
# command line, and that what a user sees — stdout, stderr, and the exit status
# for help, version, and every kind of error — is exactly what the docs promise.
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
printf '%s\n' '{"name":"cli_smoke","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","packages":[{"name":"cli","version":"=1.0.0","source":"file:packages/cli.mfp","direct":true,"requiredBy":[]}]}' > "$WORK/project.json"
cat > "$WORK/src/main.mfb" <<'MFB'
IMPORT cli
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET options AS List OF cli::Option = [cli::Option[name := "port", alias := "p", kind := cli::OptionKind.Integer, required := TRUE, help := "Port to listen on"], cli::Option[name := "offset", alias := "o", kind := cli::OptionKind.Integer, defaultValue := "99"], cli::Option[name := "ratio", kind := cli::OptionKind.Float, defaultValue := "0.5"], cli::Option[name := "enabled", alias := "b", kind := cli::OptionKind.Bool], cli::Option[name := "title", alias := "t", defaultValue := "FALLBACK"], cli::Option[name := "mode", choices := ["fast", "slow"], defaultValue := "fast"], cli::Option[name := "include", alias := "I", repeatable := TRUE], cli::Option[name := "verbose", alias := "v", kind := cli::OptionKind.Flag, repeatable := TRUE], cli::Option[name := "quiet", alias := "q", kind := cli::OptionKind.Flag]]
  LET command AS cli::Command = cli::Command[summary := "A smoke test.", version := "1.2.3", operands := "[file...]", options := options, footer := "footer"]
  LET args AS cli::Args = cli::parse(command)
  IF cli::getInteger(args, "port") = 0 THEN cli::usageError(command, "port 0 is not allowed")
  io::print("port=" & toString(cli::getInteger(args, "port")))
  io::print("offset=" & toString(cli::getInteger(args, "offset")))
  io::print("ratio=" & toString(cli::getFloat(args, "ratio")))
  io::print("enabled=" & toString(cli::getBool(args, "enabled")))
  io::print("title=[" & cli::getString(args, "title") & "] given=" & toString(cli::has(args, "title")))
  io::print("mode=" & cli::getString(args, "mode"))
  io::print("include=" & strings::join(cli::getStrings(args, "include"), ","))
  io::print("verbose=" & toString(cli::count(args, "verbose")) & " quiet=" & toString(cli::getFlag(args, "quiet")))
  io::print("operands=" & strings::join(cli::operands(args), ","))
  RETURN 0
END FUNC
MFB
"$MFB" build -q "$WORK"
BIN="$WORK/build/cli_smoke.out"

# Run BIN, capturing stdout, stderr, and the exit status separately.
run() {
  set +e
  "$BIN" "$@" >"$WORK/out" 2>"$WORK/err"
  status=$?
  set -e
  out="$(cat "$WORK/out")"
  err="$(cat "$WORK/err")"
}

# The command line parses: exit 0, nothing on stderr, and stdout holds $1.
accepts() {
  local want="$1"; shift
  run "$@"
  [[ "$status" -eq 0 ]] || fail "exit $status for [$*]; stderr: $err"
  [[ -z "$err" ]] || fail "unexpected stderr for [$*]: $err"
  [[ "$out" == *"$want"* ]] || fail "for [$*]
  expected a line: $want
  got:
$out"
}

# The command line is refused: exit 2, nothing on stdout, and exactly the
# documented two lines on stderr.
refuses() {
  local message="$1"; shift
  run "$@"
  [[ "$status" -eq 2 ]] || fail "expected exit 2 for [$*], got $status; stdout: $out; stderr: $err"
  [[ -z "$out" ]] || fail "unexpected stdout for [$*]: $out"
  local want="cli_smoke: $message
Try 'cli_smoke --help' for more information."
  [[ "$err" == "$want" ]] || fail "for [$*]
  expected stderr:
$want
  got:
$err"
}

# --help: stdout, exit 0, even though the required --port is missing.
expected_help="A smoke test.

Usage: cli_smoke [options] [file...]

Options:
  -p, --port <port>         Port to listen on [required]
  -o, --offset <offset>     [default: 99]
      --ratio <ratio>       [default: 0.5]
  -b, --enabled <enabled>
  -t, --title <title>       [default: FALLBACK]
      --mode <mode>         [possible values: fast, slow] [default: fast]
  -I, --include <include>...
  -v, --verbose...
  -q, --quiet
  -h, --help                Print help
  -V, --version             Print version

footer"
for spelled in --help -h -qh; do
  run "$spelled"
  [[ "$status" -eq 0 ]] || fail "$spelled exited $status; stderr: $err"
  [[ -z "$err" ]] || fail "$spelled wrote to stderr: $err"
  [[ "$out" == "$expected_help" ]] || fail "$spelled printed:
$out
  expected:
$expected_help"
done

# --version: one line, exit 0.
for spelled in --version -V; do
  run "$spelled"
  [[ "$status" -eq 0 && "$out" == "cli_smoke 1.2.3" && -z "$err" ]] || fail "$spelled: status $status, stdout [$out], stderr [$err]"
done

# The program name comes from how the program was invoked, without `.out`.
set +e
renamed="$(exec -a /usr/local/bin/serve "$BIN" --version)"
set -e
[[ "$renamed" == "serve 1.2.3" ]] || fail "argv[0] /usr/local/bin/serve printed [$renamed]"

# An argv[0] that is empty or not valid UTF-8 falls back to the executable's
# own file name instead of crashing help, version, or an error report.
for argv0 in "" $'\xff'; do
  set +e
  named="$(exec -a "$argv0" "$BIN" --version 2>&1)"
  set -e
  [[ "$named" == "cli_smoke 1.2.3" ]] || fail "argv[0] [$argv0] printed [$named]"
done

# Defaults, and every spelling of a value.
accepts 'port=3000'            -p 3000
accepts 'port=3000'            --port 3000
accepts 'port=3000'            --port=3000
accepts 'port=3000'            -p3000
accepts 'port=3000'            -p=3000
accepts 'offset=99'            -p 1
accepts 'ratio=0.50'           -p 1
accepts 'enabled=FALSE'        -p 1
accepts 'title=[FALLBACK] given=FALSE' -p 1
accepts 'mode=fast'            -p 1

# Every accepted boolean spelling.
for yes in true TRUE yes on 1; do accepts 'enabled=TRUE' -p 1 -b "$yes"; done
for no in false False no off 0; do accepts 'enabled=FALSE' -p 1 --enabled="$no"; done

# Negative numbers are values, separately or inline.
accepts 'offset=-3'            -p 1 --offset -3
accepts 'offset=-3'            -p 1 --offset=-3
accepts 'ratio=-2.50'          -p 1 --ratio -2.5

# Inline values keep what follows the first `=`, and an empty value is a value.
accepts 'title=[a=b] given=TRUE' -p 1 --title=a=b
accepts 'title=[] given=TRUE'    -p 1 --title=
accepts 'title=[] given=TRUE'    -p 1 -t ''
accepts 'title=[-x] given=TRUE'  -p 1 --title=-x

# Repeatable options, counted flags, and grouped short flags.
accepts 'include=a,b,c'        -p 1 -I a --include=b -Ic
accepts 'verbose=3 quiet=TRUE' -p 1 -vvq -v
accepts 'verbose=0 quiet=FALSE' -p 1

# Operands appear anywhere; `-` is one; everything after `--` is one.
accepts 'operands=a.txt,-,b.txt' a.txt -p 1 - b.txt
accepts 'operands=x,-p,--nope,--' -p 1 -- x -p --nope --

# Everything the parser refuses: exit 2 with the exact message.
refuses "option '-p' requires a value"                        -p
refuses "option '--title' requires a value; write --title=-v to pass a value starting with '-'" -p 1 --title -v
refuses "missing required option '--port'"
refuses "unknown option '--nope'"                             -p 1 --nope
refuses "unknown option '-x'"                                 -p 1 -vx
refuses "option '--port' was given more than once"            -p 1 -p 2
refuses "option '--quiet' does not take a value"              -p 1 --quiet=true
refuses "invalid value 'maybe' for '--enabled': expected true or false" -p 1 -b maybe
refuses "invalid value 'twelve' for '--offset': expected an integer"    -p 1 -o twelve
refuses "invalid value '99999999999999999999' for '--offset': integer out of range" -p 1 -o 99999999999999999999
refuses "invalid value 'half' for '--ratio': expected a number" -p 1 --ratio half
refuses "invalid value 'medium' for '--mode': expected one of fast, slow" -p 1 --mode medium
refuses "option '--help' does not take a value"               --help=1
refuses "unknown option '--nope'"                             --nope --help

# An argument that is not valid UTF-8 is a usage error, not a crash.
refuses "an argument is not valid UTF-8 text"                 -p 1 $'\xff'
refuses "an argument is not valid UTF-8 text"                 $'--title=\xc3'

# A program's own check reports the same way through cli::usageError.
refuses "port 0 is not allowed"                               -p 0

echo "cli smoke passed"

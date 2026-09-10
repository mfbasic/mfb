#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MFB="${1:-$ROOT/target/debug/mfb}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

"$MFB" build -q "$ROOT/packages/cli"
mkdir -p "$WORK/src" "$WORK/packages"
cp "$ROOT/packages/cli/cli.mfp" "$WORK/packages/cli.mfp"
printf '%s\n' '{"name":"cli_smoke","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","packages":[{"name":"cli","version":"=0.1.0","source":"file:packages/cli.mfp"}]}' > "$WORK/project.json"
printf '%s\n' 'IMPORT cli' 'IMPORT io' '' 'FUNC main() AS Integer' '  LET options AS List OF cli::Option = [cli::Option[name := "port", alias := "p", required := TRUE, kind := cli::OptionKind.Integer], cli::Option[name := "enabled", alias := "b", required := FALSE, kind := cli::OptionKind.Bool]]' '  cli::parse(options)' '  io::print(toString(cli::getInteger("port", 0)))' '  io::print(toString(cli::getBool("enabled", FALSE)))' '  RETURN 0' 'END FUNC' > "$WORK/src/main.mfb"
"$MFB" build -q "$WORK"
BIN="$WORK/build/cli_smoke.out"
[[ "$("$BIN" -p 3000 -b true)" == $'3000\nTRUE' ]]
[[ "$("$BIN" -p 3000 -b 0)" == $'3000\nFALSE' ]]
set +e
"$BIN" -p >/dev/null 2>&1
STATUS=$?
set -e
[[ "$STATUS" -ne 0 ]]

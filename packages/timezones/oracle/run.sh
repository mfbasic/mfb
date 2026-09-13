#!/usr/bin/env bash
#
# run.sh — build the package and the probe, then compare modes against zoneinfo.
#
#     ./run.sh                     # every mode, with the default compiler
#     ./run.sh path/to/mfb         # every mode, with a compiler you name
#     ./run.sh '' offsets          # only some modes
#
# Exit status is 0 iff every mode agreed, apart from divergences declared in
# divergences.json.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
MFB="${1:-}"
if [ -z "$MFB" ]; then
  MFB="$ROOT/target/release/mfb"
fi
shift || true
MODES=("$@")
if [ "${#MODES[@]}" -eq 0 ]; then
  MODES=(offsets civil roundtrip ixdtf)
fi

# `set -e` does not abort on a failing `[[ ]]` under the bash 3.2 that macOS
# ships, so every check says so itself.
fail() { echo "FAIL: $*" >&2; exit 1; }

if [ ! -x "$MFB" ]; then
  fail "no compiler at $MFB — pass one as \$1, or run: cargo build --release --bin mfb"
fi

echo "==> Building packages/timezones"
"$MFB" build -q "$ROOT/packages/timezones"

echo "==> Building the probe"
mkdir -p "$HERE/probe/packages"
cp "$ROOT/packages/timezones/timezones.mfp" "$HERE/probe/packages/timezones.mfp"
"$MFB" build -q "$HERE/probe"

if [ ! -x "$HERE/.venv/bin/python" ]; then
  echo "==> Creating oracle/.venv"
  python3 -m venv "$HERE/.venv"
fi
"$HERE/.venv/bin/pip" install -q --disable-pip-version-check -r "$HERE/requirements.txt"
PY="$HERE/.venv/bin/python"

mkdir -p "$HERE/jobs"
status=0
for mode in "${MODES[@]}"; do
  echo "==> Mode $mode"
  started=$(date +%s)
  if [ "$mode" = "ixdtf" ]; then
    # Temporal is the reference here, and it keeps only the candidates its own
    # (older) tz release agrees on. Every skip is logged with its reason.
    if ! command -v node >/dev/null 2>&1; then
      fail "ixdtf mode needs Node 24 or newer (Temporal behind --harmony-temporal)"
    fi
    "$PY" "$HERE/corpus.py" ixdtf > "$HERE/jobs/ixdtf.candidates"
    node --harmony-temporal "$HERE/temporal.mjs" filter "$HERE/jobs/ixdtf.candidates" \
      > "$HERE/jobs/ixdtf.txt" 2> "$HERE/jobs/ixdtf.skipped"
    echo "    $(wc -l < "$HERE/jobs/ixdtf.skipped" | tr -d ' ') candidates skipped (jobs/ixdtf.skipped)"
    node --harmony-temporal "$HERE/temporal.mjs" answer "$HERE/jobs/ixdtf.txt" > "$HERE/jobs/ixdtf.expected"
  else
    "$PY" "$HERE/corpus.py" "$mode" > "$HERE/jobs/$mode.txt"
    "$PY" "$HERE/oracle.py" "$HERE/jobs/$mode.txt" > "$HERE/jobs/$mode.expected"
  fi
  "$HERE/probe/build/tzprobe.out" "$HERE/jobs/$mode.txt" > "$HERE/jobs/$mode.actual" \
    || fail "the probe exited non-zero on $mode: the probe broke, not a disagreement"
  if ! "$PY" "$HERE/diff.py" "$mode"; then
    status=1
  fi
  echo "    $(wc -l < "$HERE/jobs/$mode.txt" | tr -d ' ') jobs, $(( $(date +%s) - started )) s"
done
exit "$status"

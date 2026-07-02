#!/bin/sh
# runtime_ulp.py --runner for the x86-64 test boxes (.ai/remote_systems.md):
#   ULP_PORT=2227 [ULP_FLAVOR=musl]  ./run-remote-x86.sh <executable>   # Alpine musl
#   ULP_PORT=2228 ULP_FLAVOR=glibc   ./run-remote-x86.sh <executable>   # Debian glibc
# The harness hands over the last "Wrote executable" path (the musl flavor);
# ULP_FLAVOR=glibc swaps to the sibling -glibc.out. Relays stdout + exit code.
PORT="${ULP_PORT:?set ULP_PORT (2227 alpine musl / 2228 debian glibc)}"
BIN="$1"
case "${ULP_FLAVOR:-musl}" in
  glibc) BIN="$(printf '%s' "$BIN" | sed 's/-musl\.out$/-glibc.out/')" ;;
esac
scp -q -P "$PORT" -o StrictHostKeyChecking=no "$BIN" test@127.0.0.1:/tmp/ulp-run.bin >/dev/null 2>&1 || exit 120
ssh -p "$PORT" -o StrictHostKeyChecking=no test@127.0.0.1 'chmod +x /tmp/ulp-run.bin && /tmp/ulp-run.bin'

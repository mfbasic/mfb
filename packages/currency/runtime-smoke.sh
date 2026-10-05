#!/usr/bin/env bash
#
# runtime-smoke.sh — prove the package links and that its rates table behaves
# as documented across threads. Build an executable that imports `currency`
# against the compiled .mfp, run it, and check what it printed.
#
# The package's own tests (`mfb test packages/currency`) run inside the package
# on one thread. This is the only check that a consumer can hold the
# currency::Amount the package hands back, and that the rates table is per
# thread: a worker starts with an EMPTY table, and exportRates -> importRates
# carries the main thread's rates into it.
#
#     ./runtime-smoke.sh [path/to/mfb]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MFB="${1:-$ROOT/target/release/mfb}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# `set -e` does not abort on a failing `[[ ]]` under the bash 3.2 that macOS
# ships, so every assertion says so itself. Never write a bare `[[ ]]` here.
fail() { echo "FAIL: $*" >&2; exit 1; }

"$MFB" build -q "$ROOT/packages/currency"
mkdir -p "$WORK/src" "$WORK/packages"
cp "$ROOT/packages/currency/currency.mfp" "$WORK/packages/currency.mfp"
printf '%s\n' '{"name":"currency_runtime_smoke","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","packages":[{"name":"currency","version":"=0.1.0","source":"file:packages/currency.mfp","direct":true,"requiredBy":[]}]}' > "$WORK/project.json"
cat > "$WORK/src/main.mfb" <<'MFB'
IMPORT currency
IMPORT io
IMPORT thread

' Report what the worker's own table holds before and after importing the
' parent's rates, and convert with the imported rates.
ISOLATED FUNC worker(self AS ThreadWorker OF String TO String, saved AS Map OF String TO Fixed) AS String
  LET before AS String = toString(currency::hasRate("EUR"))
  currency::importRates(saved)
  LET dollars AS currency::Amount = currency::convert(currency::amount(10m, "eur"), "USD")
  RETURN before & " " & toString(currency::hasRate("EUR")) & " " & currency::format(dollars)
END FUNC

FUNC main() AS Nothing
  currency::setRate("USD", 1F)
  currency::setRate("EUR", 1.25F)
  LET total AS currency::Amount = currency::add(currency::amount(5m, "usd"), currency::amount(4m, "EUR"))
  io::print(currency::format(total))
  RES t AS Thread OF String TO String = thread::start(worker, currency::exportRates())
  io::print(thread::waitFor(t))
  LET failed AS String = currency::format(currency::add(total, currency::amount(1m, "JPY"))) TRAP(e)
    RECOVER toString(e.code)
  END TRAP
  io::print(failed)
END FUNC
MFB
"$MFB" build -q "$WORK"
(cd "$WORK" && "$WORK/build/currency_runtime_smoke.out" > "$WORK/output.txt")

lines="$(wc -l < "$WORK/output.txt" | tr -d ' ')"
if [ "$lines" != 3 ]; then
  fail "expected 3 lines of output, got $lines:
$(cat "$WORK/output.txt")"
fi
line1="$(sed -n '1p' "$WORK/output.txt")"
line2="$(sed -n '2p' "$WORK/output.txt")"
line3="$(sed -n '3p' "$WORK/output.txt")"

# 5 USD + 4 EUR at 1.25 = 10 USD, in the first amount's currency.
if [ "$line1" != "10.00 USD" ]; then
  fail "add across currencies is wrong: $line1"
fi

# The worker's table starts empty, and importRates fills it: 10 EUR = 12.50 USD.
if [ "$line2" != "FALSE TRUE 12.50 USD" ]; then
  fail "the worker's rates table is wrong: $line2"
fi

# JPY has no rate on the main thread: currency::ErrorUnknownCurrency.
if [ "$line3" != "93170003" ]; then
  fail "a currency with no rate did not fail with ErrorUnknownCurrency: $line3"
fi

echo "currency runtime smoke passed"

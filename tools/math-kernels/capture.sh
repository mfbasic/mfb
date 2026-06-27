#!/usr/bin/env bash
#
# capture.sh — drive gen_inputs.py through the macOS-libm oracle (capture_ref)
# and write one committed reference-vector file per transcendental.
#
# This is the "run once on the reference macOS" step from plan-01-simd Phase 5.
# It MUST be run on the reference Mac (Darwin/aarch64): the oracle links the
# system libm, so the captured expected_bits ARE macOS libm. The resulting files
# are committed and read by the Rust kernel tests on every target — Linux/CI
# never recapture, they validate against these.
#
# Each output file carries a provenance header (OS + libm/system version, the
# generator command, capture date) so the pin is auditable, mirroring the
# generated-Unicode-table precedent (scripts/gen_regex_unicode.py).
#
# Usage:
#   ./capture.sh [OUT_DIR]
# OUT_DIR defaults to ./reference (a tool-local sample). For a real capture the
# implementer points it at the in-tree test data location, e.g.:
#   ./capture.sh ../../tests/_data/math_kernel_ref
# (kept out of src/ here so this tool never writes outside tools/ by default).

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="${1:-$HERE/reference}"

FUNCS=(exp log log10 sin cos tan asin acos atan atan2 pow)

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "capture.sh: refusing to run on $(uname -s) — the reference oracle is" >&2
  echo "  macOS libm and must be captured on the reference Mac (Darwin)." >&2
  exit 1
fi

# Build the oracle fresh so the binary always matches the committed source.
echo "building capture_ref…" >&2
cc -O0 -std=c11 -Wall -Wextra -o "$HERE/capture_ref" "$HERE/capture_ref.c" -lm

mkdir -p "$OUT_DIR"

# Provenance facts shared by every file.
OS_VER="$(sw_vers -productName 2>/dev/null) $(sw_vers -productVersion 2>/dev/null) ($(sw_vers -buildVersion 2>/dev/null))"
KERNEL="$(uname -smr)"
# libm ships inside libSystem; record its identity so the pin is reproducible.
LIBM_PATH="/usr/lib/system/libsystem_m.dylib"
if [[ -e "$LIBM_PATH" ]]; then
  LIBM_ID="$(otool -L "$LIBM_PATH" 2>/dev/null | awk 'NR==2{print $1" "$2" "$3}')"
else
  LIBM_ID="libSystem (shared cache; path not on disk)"
fi
CC_VER="$(cc --version 2>/dev/null | head -1)"
DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

for fn in "${FUNCS[@]}"; do
  out="$OUT_DIR/$fn.ref"
  {
    echo "# GENERATED — macOS libm reference vectors for math::$fn (NEON kernel oracle)."
    echo "# Source: tools/math-kernels/capture.sh  (capture_ref.c + gen_inputs.py)"
    echo "# Oracle:  macOS system libm — values are $fn() as computed by:"
    echo "#   OS:     $OS_VER"
    echo "#   Kernel: $KERNEL"
    echo "#   libm:   $LIBM_ID"
    echo "#   cc:     $CC_VER"
    echo "#   Date:   $DATE (UTC)"
    echo "# Each NEON f64 kernel result must be within <=1 ULP of expected_bits."
    if [[ "$fn" == "atan2" || "$fn" == "pow" ]]; then
      echo "# Format: <x_bits> <y_bits> <expected_bits>  (lowercase IEEE-754 hex)"
    else
      echo "# Format: <x_bits> <expected_bits>  (lowercase IEEE-754 hex)"
    fi
    python3 "$HERE/gen_inputs.py" "$fn" 2>/dev/null | "$HERE/capture_ref" "$fn"
  } > "$out"
  count="$(grep -vc '^#' "$out")"
  echo "  $fn -> $out ($count vectors)" >&2
done

echo "done: ${#FUNCS[@]} reference files in $OUT_DIR" >&2

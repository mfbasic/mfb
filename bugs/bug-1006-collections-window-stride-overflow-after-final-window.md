# bug-1006: `collections::window` overflows while advancing past its last window for a large valid stride

Last updated: 2026-10-07
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: `tests/rt-behavior/collections/window-large-stride-rt` (proposed)

`collections::window` accepts any positive `Integer` stride. If at least one window fits but the stride is so large that adding it to the current start index overflows, the function raises an arithmetic-overflow error instead of returning the windows it already found. For a two-element list and size 1, stride `9223372036854775807` should produce exactly one window.

**Correct behavior:** a positive stride larger than the remaining list length ends iteration after the current window; it must not overflow or discard the valid result.

## Failing Reproduction

Use a runtime value to keep the input dynamic and exercise the generic MFBASIC implementation:

```
IMPORT collections
IMPORT io
IMPORT os

FUNC main() AS Integer
  LET stride AS Integer = toInt(os::getEnvOr("STRIDE", "9223372036854775807"))
  LET windows = collections::window(["a", "b"], 1, stride)
  io::print(toString(len(windows)))
  io::print(collections::get(collections::get(windows, 0), 0))
  RETURN 0
END FUNC
```

Build with the current compiler and run with `STRIDE=9223372036854775807` (or leave the default):

- Observed on macOS aarch64, compiler built from HEAD `9d2240565`: exits 255 with `Error: 7-705-0010` / `Arithmetic overflow or numeric conversion outside the destination range.`
- Expected: prints `1` then `a`, and exits 0. A stride of 1 over the same list still produces two windows.

## Root Cause

`src/codegen/builtins/collections/func_window.rs`, `BODY` (`__collections_window`): after appending the window at `i = 0`, the body advances to `i = INT64_MAX`. The next loop condition evaluates `i + size`, which overflows before it can establish that no further window fits. The API only rejects `size < 1` or `stride < 1`; every positive stride is valid. The constant fast paths in `window_fast_path` currently require literal size and stride, so this reproduction reaches the generic MFBASIC body.

## Blast Radius

- `collections::window` — observed failure in the generic implementation.
- Constant-stride native fast paths in `window_fast_path` — currently only fixed-width stride 1 or string inputs are accelerated; audit their index advancement if the supported fast-path domain expands.
- `collections::chunks` — uses a similar `i + chunkSize` / `i + chunkSize` progression in its generic body, but a positive chunk size larger than the remaining length clamps the end and then terminates; this specific last-window stride case does not apply.

## Fix Design

Make the loop condition overflow-safe (for example, compare `size` with the remaining length after confirming `i <= len(value)`) so an index beyond the input terminates normally. Do not reject a positive stride solely because it leaves the next start beyond the input.

## Validation Plan

- Regression: the program above returns one window for `INT64_MAX`; stride 1 still returns two.
- Runtime proof: execute the built program and check both output and exit status.
- Doc sync: none expected; the fix follows the documented positive-stride contract.
- Full suite: acceptance and runtime suites after implementation.

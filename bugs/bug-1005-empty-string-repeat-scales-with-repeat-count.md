# bug-1005: `strings::repeat("", n)` spends O(n) time copying zero bytes

Last updated: 2026-10-07
Effort: small (<1h)
Severity: LOW
Class: Correctness / Performance

Status: Open
Regression Test: `tests/rt-behavior/strings/repeat-empty-large-count-rt` (proposed)

`strings::repeat` documents that repeating an empty string produces the empty string for any valid nonnegative count. The current native lowering still runs its outer loop once per requested copy even when the source length is zero. Consequently, `strings::repeat("", 1000000000000)` does not return promptly, despite producing no bytes and requiring no allocation proportional to the count. `astrings::repeat` has a second loop over the requested count to duplicate attribute spans; when the attributed text is empty, that loop also does no useful work but still scales with `times`.

**Correct behavior:** for every nonnegative `times`, repeating an empty string returns `""` in time independent of `times` (after ordinary argument validation).

## Failing Reproduction

Build the following program and run it with a two-second process timeout:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET x AS String = strings::repeat("", 1000000000000)
  io::print("[" & x & "]")
  RETURN 0
END FUNC
```

- Observed on macOS aarch64, compiler built from HEAD `9d2240565`: execution did not finish within two seconds.
- Expected: output `[]` and exit status 0 promptly. `strings::repeat("", 0)` and `strings::repeat("", 1)` both have the same expected value.

## Root Cause

`src/codegen/builtins/strings/func_repeat.rs`, `lower`: after measuring `total = byteLen(value) * times` and allocating the result, the copy lowering enters `strings_repeat_outer` while `times_rem != 0`. When `len == 0`, the inner loop copies nothing, but the outer loop still decrements `times_rem` one at a time. The result allocation and terminator are constant-sized; only this needless loop makes runtime proportional to `times`.

The attributed-string wrapper independently has the same shape: `src/codegen/builtins/astrings/helper_repeat.rs`, `__astrings_repeat`, loops while `k < times` and walks `spans` each pass. Empty text has zero scalars and no spans, so after the underlying string repeat is made constant-time, this wrapper would still spend O(times) doing nothing unless it also handles the empty case.

## Blast Radius

- `strings::repeat` — observed slow path; handle the zero-byte source before entering the copy loop.
- `astrings::__astrings_repeat` — both delegates text repetition and independently loops to duplicate spans; an empty attributed string with a large count incurs the delay in both places (`src/codegen/builtins/astrings/helper_repeat.rs`, `__astrings_repeat`).
- In-place string self-repeat — separate lowering in `src/codegen/collection/assign/string_self_update.rs`, `emit_string_grow_write`; audit for the same empty-prefix loop before deciding scope.

## Fix Design

Branch around the repeat-copy loop when the measured source byte length is zero, while preserving the empty owned-string result and existing negative-count and allocation error behavior. Add the large-count runtime regression and a contrast with a nonempty value at a small count.

## Validation Plan

- Regression: the program above terminates and prints `[]`; `strings::repeat("x", 3)` remains `xxx`.
- Runtime proof: execute the generated program under a bounded timeout.
- Doc sync: none expected; this restores the documented empty-input behavior.
- Full suite: acceptance and runtime suites after implementation.

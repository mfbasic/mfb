# bug-1104: `collections::window` with a run-time stride near the Integer maximum raises an undeclared `ErrOverflow`, located inside the stdlib body

Last updated: 2026-10-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (declared errors)

Status: Fixed
Regression Test: `tests/runtime/rt_collection_chunks_window_sizing.rs` (`runtime_stride_near_integer_maximum_returns_the_windows_that_fit`)

## Cause (coordinator, bug-staging-24)

- Rule: a builtin raises only the errors it declares (`ErrOutOfMemory`, `ErrInvalidArgument` for `window`), located at the caller; `window` with any positive stride returns the windows that fit, as its fast path does since bug-1008.
- Where: the generic `__collections_window` body advances with `i = i + stride`, which overflows before the loop test.
- Fix: the generic body's loop uses the fast path's overflow-free test (`start <= n - size`, step only while `stride <= (n - size) - start`), so no overflow can arise; nothing special-cased by stride value.

Found by bs24-agent-e while fixing bugs 1007/1008, reproduced by the bug-staging-24
coordinator, and placed in a later phase.

**The single correct behavior a fix produces:** the program below prints `1` (one window,
`["a"]`), as the native fast path now does for the same constant stride, and as
`mfb man collections window` describes; no error escapes that `window` does not declare
(`ErrOutOfMemory`, `ErrInvalidArgument`).

## Reproduction (main `7bb985ed8`)

```
IMPORT collections
IMPORT io
IMPORT os

FUNC main() AS Nothing
  LET st AS Integer = toInt(os::getEnvOr("STRIDE", "9223372036854775807"))
  LET w AS List OF List OF String = collections::window(["a", "b", "c"], 1, st)
  io::print(toString(len(w)))
END FUNC
```

```
Error:    7-705-0010
Message:  Arithmetic overflow or numeric conversion outside the destination range.
Location: src/main.mfb:280:11
```

The location is a line of the assembled stdlib source, not the caller's line 7.

## Where

The generic `__collections_window` body advances with `i = i + stride`, which overflows
before the loop test sees the end of the list. bug-1008's fix made the native fast path's
loop overflow-free (`start <= n - size`, step only while `stride <= (n - size) - start`); the
generic body needs the same test.

## Fix

`__collections_window` (`src/codegen/builtins/collections/func_window.rs`) now computes
`last = len(value) - size` once, loops `WHILE i <= last`, and exits before stepping when
`stride > last - i`, so neither `i + size` nor `i + stride` is formed past the end (the fast
path's test). RED: the new test failed on the unfixed body (run-time stride `Integer` max);
GREEN after. The `DESC` sentence describing the loop condition was reworded.

Goldens expected to move (stdlib body assembled into the IR; `.ir` kind only, plus the
`.ncode` sha of the byte-identity fixture): `byte-identity/collections/collections_codegen_cover_rt`
(`.ir` + five `.ncodesum`/ncode shas), `rt-behavior/collections/{window-string-native-rt,
collections-artifact-coverage-rt,collection-accelerator-declines-rt}` and
`rt-behavior/resources/bug643-collection-helpers-keep-caller-handles-rt` (`.ir`). Measured:
`artifact-gate collections` 6 diffs, `test-accept` over those four fixtures 4 mismatches, all
in the `__collections_window` body.

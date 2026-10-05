# bug-838: `ErrStackOverflow` carries `source.line = 0`, `source.char = 0` — no usable origin location

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness (diagnostic metadata)

Status: Open
Regression Test: tests/rt-behavior/<recursion>/stack-overflow-error-source-rt (to add)

**Owner decision (2026-10-05):** `ErrStackOverflow`'s `source` is the actual location of the call that overflowed. In
the repro that is line 3, the recursive call site, not line 2 (the `FUNC` definition) and
never `0:0`.

When unbounded recursion raises `ErrStackOverflow` (7-701-0002), the caught `Error` has
`e.source.filename = "src/main.mfb"` but `e.source.line = 0` and `e.source.char = 0`.
That is true for direct recursion, for mutual recursion called through a function value,
and on a worker thread. Every other runtime-generated error tested in this session
carries a real location: the division at 44:25, toInt at 55:11, error() at 9:8, and
package errors at lib.mfb:6:22.

The single correct behavior is an open owner decision. The error should carry a real
`ErrorLoc`: either the declaration line of the function whose entry check fired, or the
call site that could not be entered. It should never be line 0.

This is classed UNCLEAR. The spec says where the error is raised, but not what
`e.source` must hold for it.

References:

- `src/docs/spec/language/04_types.md` §4.4: "A runtime-generated error (divide-by-zero,
  overflow, a failing built-in or package helper) carries the source location of the
  failing expression."
- `src/docs/spec/language/06_functions.md` (Recursion): "raises `ErrStackOverflow`
  (7-701-0002) at the entry of the function being called, before it binds anything".
- bugs/completed/bug-789-recursion-exhaustion-crashes-native-program.md (introduced the check).

## Failing Reproduction

```basic
IMPORT io
FUNC down(n AS Integer) AS Integer
  RETURN down(n + 1) + 1
END FUNC
FUNC main() AS Nothing
  LET a = down(0) TRAP(e)
    io::print(toString(e.code) & "@" & e.source.filename & ":" & toString(e.source.line) & ":" & toString(e.source.char))
    RECOVER 0
  END TRAP
END FUNC
```
Built and run in /tmp/hardening/B01/t/em1 (the same program with more cases).

- Observed: `77010002@src/main.mfb:0:0`
- Expected: a nonzero line, either `2` (the `FUNC down` entry) or `3` (the recursive call site).

## Root Cause

`src/codegen/runtime/stack_limit.rs:emit_stack_overflow_block` calls
`raise_error_bare("ErrStackOverflow")`
(`src/codegen/error/emission/builder_error_emission.rs`). That happens in a cold block
reached from the function prologue, where the builder's current source location has not
been set to any statement yet. The stamped `ErrorLoc` is therefore line/char 0.

## Goal

- `e.source.line` is nonzero and names the function's declaration line (recommended:
  available in the prologue with no call-site plumbing).

### Non-goals

- The raise point (entry of the callee, caller's TRAP catches) must not change.

## Validation Plan

- An rt fixture printing `e.source`. Then the full suite. Then document the chosen
  location in 06_functions.md.

## Open Decisions

- Callee declaration line (recommended) vs. caller call site. Also decide whether a
  `stack_limit` spec sentence should state the choice.

## Summary

This is metadata only. Control flow is unaffected.

# bug-302: `json::parse` crashes (SIGSEGV) on untrusted input via unbounded native recursion in scalar scanners

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: HIGH
Class: Robustness (DoS on untrusted input)

Status: Open
Regression Test: tests/rt-error (new) — `json::parse` of a long whitespace run / long number returns an error, never crashes

The `json_package.mfb` array and object parsers were deliberately rewritten to be
iterative ("to avoid deep call stacks on large objects"), but the scalar scanners
were left recursive: `__json_skipWhitespace`, `__json_consumeDigits`,
`__json_collectNumber`, `__json_isRawControlCharAt`, and `__json_expectLiteralAt`
each recurse once per input character (`RETURN __json_skipWhitespace(chars, index +
1)`). MFBASIC has no tail-call optimization, so recursion depth equals input length
— a long whitespace run or long number overflows the native stack and crashes the
process. JSON is the untrusted-data boundary (and the HTTP server dispatches handler
bodies through it), so a ~200 KB payload — far under the 64 MiB request cap — is a
remote crash.

The single correct behavior a fix produces: `json::parse` of any input (including
long whitespace/number runs and deeply nested structures) either parses or returns a
clean error, never a stack-overflow crash.

References:

- Iterative array/object parsers with the depth-hazard comments at
  `src/builtins/json_package.mfb:348, :393`.
- Found during goal-06 review of `src/builtins/json_package.mfb`.

## Failing Reproduction

```
' json::parse(<200000 spaces> & "1")
' json::parse("1" & <100000 zeros>)
```

- Observed: exit 139 (SIGSEGV) for both.
- Expected: successful parse (or a bounded error), no crash.

## Root Cause

`src/builtins/json_package.mfb:741` (`__json_skipWhitespace`), `:711`
(`__json_consumeDigits`), `:597` (`__json_collectNumber`), `:484`
(`__json_isRawControlCharAt`), `:726` (`__json_expectLiteralAt`): per-character
self-recursion with no TCO → stack depth = input length.

## Goal

- Convert the recursive scalar scanners to `WHILE` loops (as the array/object
  parsers already are).
- Add a nesting-depth cap to `__json_parseValue` for structural depth.

### Non-goals (must NOT change)

- The parse results for valid input.
- The iterative array/object parsers (already correct).

## Blast Radius

- The five recursive scanners (plus `__json_escapeRawControlCharAt`, same pattern) —
  fixed here.
- `__json_parseValue` structural recursion — add a depth cap.
- The HTTP server's JSON handler dispatch — benefits (no change needed).

## Fix Design

Rewrite each scanner as a `WHILE` loop accumulating index/state. Add a
configurable/constant nesting-depth cap in `__json_parseValue` that returns
`ErrInvalidFormat` past the limit. Rejected alternative: relying on a larger stack —
input-length-proportional recursion cannot be bounded that way.

## Phases

### Phase 1 — failing test
- [ ] rt-error tests for the two repros (crash today).
### Phase 2 — the fix
- [ ] Loop-ify the scanners; add the depth cap.
### Phase 3 — validation
- [ ] Full suite green; the repros parse/error without crashing; valid JSON
      unaffected.

## Validation Plan

- Regression: long-whitespace, long-number, deep-nesting inputs.
- Runtime proof: no SIGSEGV; large valid inputs parse.
- Doc sync: note the nesting-depth limit in the json man page.

## Summary

Untrusted JSON crashes the process because the scalar scanners recurse per
character; loop-ifying them (and capping nesting) removes the remote crash. This is
the untrusted-input boundary, so it matters for the MVP's HTTP/JSON story.

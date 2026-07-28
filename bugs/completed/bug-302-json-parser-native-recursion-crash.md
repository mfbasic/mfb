# bug-302: `json::parse` crashes (SIGSEGV) on untrusted input via unbounded native recursion in scalar scanners

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: HIGH
Class: Robustness (DoS on untrusted input)

Status: Fixed
Regression Test: tests/rt-behavior/json/json-parse-deep-scalar-scan-rt

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

## Resolution

All five scalar scanners are now loops. The crash was reproduced first — a 200 KB
whitespace run ahead of a trivial value made `json::parse` exit **139 (SIGSEGV)**,
exactly as reported — and the fix was bisected against it: reverting
`json_package.mfb` alone restores the segfault, reapplying it restores exit 0.

- `__json_skipWhitespace`, `__json_consumeDigits`, `__json_collectNumber` were the
  genuinely dangerous three: recursion depth equalled input length, so a payload far
  under the 64 MiB HTTP request cap crashed the process at the untrusted-data
  boundary.
- `__json_isRawControlCharAt` (bounded at 32) and `__json_expectLiteralAt` (bounded
  by `true`/`false`/`null`) could **not** overflow. They were converted anyway, so
  the scanner family reads one way and a future edit cannot reintroduce the shape
  next to four siblings that look like it. That is recorded in their comments rather
  than left to look like an oversight.

The fixture pushes to **1 MiB** of whitespace — five times the crashing size — plus a
4000-digit fraction that drives `collectNumber`/`consumeDigits` per character, and
then parses an ordinary document and stringifies it back. The last part matters: the
scanners were rewritten, not merely bounded, so correctness had to be shown
unchanged, not just absence of a crash.

### Golden churn is IR-shape only

Two JSON `.ir` goldens moved, because they capture the lowered IR of the stdlib
source that was rewritten. The runtime behaviour is unchanged, and that was verified
rather than assumed: `json-behavior`'s `build.log` — which embeds the program's
complete stdout — is **byte-identical** before and after. Only the IR shape differs.

A 200000-digit integer now returns a clean `77050003` parse error rather than
crashing; that is correct (it overflows `Float`), and the distinction between "errors
loudly" and "kills the process" is the whole point of the fix.

Full `cargo test` green; artifact gate 0 diffs; acceptance 1006/1006.

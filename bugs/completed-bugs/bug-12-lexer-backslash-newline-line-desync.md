# bug-12: A backslash before a newline inside a string literal embeds a literal newline and desyncs the lexer line counter

Last updated: 2026-07-08
Effort: small (<1h)

In `src/lexer.rs::lex_string`, the escape handler's fall-through arm
(`_ => value.push(escaped)`, `lexer.rs:348`) treats **any** character after a
backslash as a literal — including a newline. When a string literal has a `\` as
the last character of a physical source line, the lexer: (1) pushes a literal
`\n` into the string value, and (2) consumes that newline with `self.advance()`
(`lexer.rs:350`), which increments only `index`/`column` and **never** `self.line`
(`advance_line` at `:895` is the only path that bumps `line`). The
unterminated-string guard (`ch == '\n'`, `:303`) is bypassed because the newline
was swallowed as an escaped char.

Result: every token after that string — and thus every AST node line, every
diagnostic location, and the coverage line mapping — is off by one (more, if
several such escapes occur), and the string silently contains an embedded
newline the author did not write literally.

The single correct behavior a fix produces: a `\` immediately before a newline is
either a clean lex error (`MFB_LEX_UNTERMINATED_STRING`) or, if a
line-continuation-in-string is intended, consumes the newline via `advance_line`
so the line counter stays correct — and it never silently embeds a `\n` while
leaving `self.line` stale.

Severity LOW: the impact is diagnostic/line-metadata accuracy and a surprising
embedded newline, not runtime program control flow.

References:

- `src/lexer.rs:314-350` (`lex_string` `'\\'` branch) — `:348` (`_` arm pushes
  the escaped char), `:350` (`self.advance()` after the match).
- `src/lexer.rs:890-899` (`advance` bumps index/column only; `advance_line` is the
  sole `self.line` increment).
- `src/lexer.rs:303-312` (the `\n` unterminated-string guard that the escaped
  newline bypasses).
- Found during goal-01 review of `src/lexer.rs`.

## Failing Reproduction

```
LET a = "abc\
def"
LET b = 1
```

- Observed: the string `a` becomes `abc\ndef` (an embedded newline), and the
  `LET b = 1` line is reported one line earlier than its true source line in any
  subsequent diagnostic / AST location (`self.line` was never advanced past the
  string's internal newline). Multiple such escapes compound the offset.
- Expected: either a `MFB_LEX_UNTERMINATED_STRING` error at the `\`-newline, or a
  well-defined line continuation that keeps `self.line` correct; the value must
  not silently gain a newline while the counter is stale.

Contrast cases that are correct today:

- An **unescaped** newline in a string is caught as
  `MFB_LEX_UNTERMINATED_STRING` (`:303-312`).
- The documented escapes `\n \t \r \0 \" \\ \u{…}` are handled explicitly
  (`:321-347`) and never straddle a physical source line.

## Root Cause

The catch-all escape arm (`lexer.rs:348`) exists to pass an unknown escape's bare
character through, and the shared `self.advance()` after the match (`:350`)
assumes the consumed escaped char is never a newline. Neither assumption holds for
`\`-at-end-of-line: the escaped char *is* `\n`, so it is embedded verbatim and
consumed with the line-agnostic `advance`, leaving `self.line` behind by one.

## Goal

- A `\` immediately followed by a newline inside a string literal does not desync
  `self.line`, and does not silently embed a newline: it is either an explicit
  lex error or a defined continuation that advances the line counter.

### Non-goals (must NOT change)

- Do not change how the documented escapes (`\n \t \r \0 \" \\ \u`) lex.
- Do not change unescaped-newline handling (already a correct error).

## Blast Radius

- `lex_string` only — this is the sole place a string escape is consumed. No
  other lexer path consumes an escaped char with `advance`.

## Fix Design

In the `'\\'` branch, before the `match`, special-case `escaped == '\n'`: report
`MFB_LEX_UNTERMINATED_STRING` at the current position (simplest, matches the
"string can't cross a line" intent already enforced for unescaped newlines) — or,
if the language decides `\`-newline is an intentional line continuation, emit no
character and consume the newline with `self.advance_line()`. Recommended: treat
it as `MFB_LEX_UNTERMINATED_STRING`, since no spec feature defines in-string line
continuation and silently embedding a newline is surprising.

## Phases

### Phase 1 — failing test + audit

- [ ] Add a lexer test with a `\`-terminated line inside a string asserting the
      chosen behavior (error, or line-preserving continuation) and asserting the
      *next* token's `line` is correct.
- [x] Blast-radius audit complete (above).

Acceptance: test fails today (line off by one / embedded `\n`).
Commit: —

### Phase 2 — the fix

- [ ] Handle `escaped == '\n'` in the `'\\'` branch before the fall-through arm.

Acceptance: Phase 1 test passes; documented escapes unchanged.
Commit: —

### Phase 3 — validation

- [ ] `scripts/test-accept.sh` — confirm no golden movement except any new
      diagnostic fixture for the chosen behavior.

Acceptance: full suite green.
Commit: —

## Validation Plan

- Regression test(s): the `lex_string` line-counter test above.
- Runtime proof: build the reproduction and confirm the reported line of a later
  statement matches its true source line.
- Doc sync: if `\`-newline becomes a defined error, note it in the lexer/spec
  string-escape section.
- Full suite: `scripts/test-accept.sh`.

## Open Decisions

- Error vs. line continuation for `\`-newline — recommended: error
  (`MFB_LEX_UNTERMINATED_STRING`), since no spec feature defines in-string
  continuation.

## Summary

A one-character oversight (the catch-all escape arm plus a line-agnostic
`advance`) silently embeds a newline and desyncs `self.line`; the fix is a single
`escaped == '\n'` guard in `lex_string`.

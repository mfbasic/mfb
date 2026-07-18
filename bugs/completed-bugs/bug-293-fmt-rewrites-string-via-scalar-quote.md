# bug-293: `mfb fmt` silently rewrites string-literal contents (changing program output) when a backtick Scalar literal contains a quote

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed 2026-07-18
Regression Test: tests/ (new) — `mfb fmt` is idempotent and preserves string/scalar literals containing quote characters

`scan_line` in the formatter models comments, strings, numbers, and words, but a
backtick (Scalar literal) falls into the catch-all `_` arm. For a scalar containing
`"` (e.g. `` `"` ``), the scalar's quote opens the scanner's *string* mode, which
then closes at the next real string's opening quote; the real string's contents are
then scanned as code, and any word spelling a keyword is uppercased — mutating a
string literal that spec 05_fmt.md guarantees is preserved byte-for-byte. A scalar
containing `'` similarly makes the rest of the line scan as a comment (cosmetic
variant). Because `mfb fmt` is expected to be semantics-preserving, this silently
changes program output.

The single correct behavior a fix produces: `mfb fmt` treats a backtick Scalar
literal as an opaque token (consumed to its closing backtick), leaving surrounding
string literals and keyword casing untouched.

References:

- `src/docs/spec/tooling/05_fmt.md` (string literals preserved byte-for-byte).
- `src/docs/spec/language` §2.3 (backtick Scalar literals, incl. `` `'` ``).
- Found during goal-06 review of `src/fmt.rs`.

## Failing Reproduction

```
LET c AS Scalar = `"` : LET s AS String = "print if then"
```

- Observed: `mfb fmt` rewrites the line so the string becomes `"print IF THEN"`; the
  rebuilt program's output changes from `print if then` to `print IF THEN`.
- Expected: the line is reformatted (spacing/indent) but both literals are byte-for-
  byte preserved.

## Root Cause

`src/fmt.rs:141-301` (`scan_line`): no arm for backtick Scalar literals; the
backtick falls to `_`, so the scalar's quote/apostrophe is scanned as string/comment
delimiters, desynchronizing the scanner for the rest of the line.

## Goal

- Add a backtick arm to `scan_line` mirroring the string arm: consume until the
  closing backtick (honoring `\`-escapes) and emit one `Sig::Other`.

### Non-goals (must NOT change)

- Reformatting of the surrounding code (spacing/indent) that does not alter literals.
- String/comment scanning for lines without scalar literals.

## Blast Radius

- `scan_line` — fixed here.
- Related fmt scanner gaps (TESTING/TGROUP/TCASE indent, mid-line DOC) are tracked
  in bug-299 (formatter LOW cluster) — distinct root causes.

## Fix Design

Add a `` ` `` arm consuming to the matching backtick with escape handling. Rejected
alternative: escaping/normalizing the scalar contents — unnecessary; the formatter
just needs to treat it as opaque.

## Phases

### Phase 1 — failing test
- [ ] `mfb fmt` on the repro changes the string today (idempotency/preservation test
      fails).
### Phase 2 — the fix
- [ ] Add the backtick arm.
### Phase 3 — validation
- [ ] Full suite + fmt goldens green; repro is preserved and idempotent.

## Validation Plan

- Regression: fmt-preservation test with `` `"` `` and `` `'` `` scalars adjacent to
  strings/keywords.
- Runtime proof: rebuilt program output unchanged after `mfb fmt`.
- Doc sync: none.

## Summary

The formatter's scanner has no case for backtick Scalar literals, so a scalar quote
hijacks string/comment scanning and can rewrite a string literal. A backtick arm
fixes it; low risk.

## Resolution

`scan_line` gained a backtick arm that consumes a Scalar literal to its closing
backtick, mirroring the string arm — including backslash escapes, since `` `\`` ``
is legal and its escaped backtick must not close the token early.

Verified both ways with the report's exact reproduction:
`LET c AS Scalar = \`"\` : LET s AS String = "print if then"` was rewritten to
`"print IF THEN"` before the fix and is byte-identical after.

Test: `fmt::tests::scalar_literals_containing_quotes_do_not_desynchronize_the_scanner`,
covering the quote case, the apostrophe case (which desynchronized into comment
mode), an escaped backtick inside a scalar, and — importantly — that keywords
*outside* any literal are still normalized, so the new arm did not just stop the
scanner doing its job.

Every claim in this report checked out.

# bug-922: `mfb fmt` deletes a trailing comment on an `END LINK` line

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: repro measured by the review agent only (HEAD 28a15a153 build); the coordinator did not re-run it.
Regression Test: src/fmt.rs unit test (new), e.g. `end_link_keeps_trailing_comment`

`format_link_block` recognizes the closer from the line's first two words, then
emits the hard-coded text `END LINK` and discards the rest of the line. A trailing
`'` comment (or `REM` comment) on that line is silently deleted. The source
compiles with or without the comment, so nothing reports the loss. The fmt spec
promises "comments … are preserved", and says of LINK blocks that "all text and
casing [are] preserved" except that `END LINK` is re-cased.

**Correct behavior:** only the two words `END LINK` are re-cased and re-indented.
Everything after them on the line (whitespace + comment) is kept verbatim.

References:

- `src/docs/spec/tooling/05_fmt.md` intro ("String contents, comments, blank lines … are preserved") and *LINK blocks* ("`END LINK` … is the one token this pass re-cases").

## Failing Reproduction

Copy `tests/rt-behavior/native/native-link-free-rt` to /tmp, and change its `END LINK` line to
`end link ' keep this comment: closes the sqlite binding`.

```
mfb build            # Wrote executable … (the comment is legal)
mfb fmt              # Formatted …/src/main.mfb
grep -n -i "end link" src/main.mfb
```

- Observed: `53:END LINK`. The comment is gone.
- Expected: `53:END LINK ' keep this comment: closes the sqlite binding`.

Contrast: a trailing comment on a `LINK "sqlite3" AS sql` opener line, or on any
line inside the block, is kept (those lines are only trimmed and re-indented).
`END DOC ' x` is not treated as a closer by either the lexer or fmt (both use
`lexer::is_end_line`, which demands exactly two words), so there is no loss there.

## Root Cause

`src/fmt.rs:format_link_block`:
`if first.eq_ignore_ascii_case("END") && second.eq_ignore_ascii_case("LINK") { out.push(format!("{}END LINK", indent_str(level, indent_width))); … }`.
The words after `second` are never re-emitted.

## Goal

- The rest of the `END LINK` line after the two words survives formatting byte-for-byte, and a second run is a no-op.

### Non-goals (must NOT change)

- `END LINK` stays upper-cased, and LINK body text and casing are untouched.

## Blast Radius

- `src/fmt.rs:format_link_block`, `END LINK` arm: fixed.
- The same arm's opener/closer lines (`END FUNC`/`END FREE`/…) already emit `text` whole, so they are unaffected.

## Validation Plan

- Unit test: `LINK "x" AS y\nend link ' c\n` formats to `LINK "x" AS y\nEND LINK ' c\n`.
- Doc sync: none beyond the existing contract.

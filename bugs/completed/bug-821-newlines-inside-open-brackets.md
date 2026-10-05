# bug-821: a list, map or record literal cannot span lines without `_`

Last updated: 2026-10-04
Effort: medium (guess): lexer/parser, spec, `mfb fmt`
Severity: MEDIUM: long literals (tables, code lists) are unwritable without `_` on every line
Class: Language (surface syntax)

Status: Closed (2026-10-04)
Regression Test: `tests/rt-behavior/lexical/literal-spans-lines-valid` (every form
below, each compared at runtime with its one-line twin);
`tests/syntax/lexical/unclosed-bracket-eof-invalid`,
`tests/syntax/lexical/unclosed-bracket-before-statement-invalid`,
`tests/syntax/lexical/bracket-comma-invalid`; lexer unit tests
(`src/lexer.rs`, `newline_inside_open_brackets_is_not_a_separator` and siblings);
fmt unit tests (`src/fmt.rs`, `multi_line_bracket_layouts_are_preserved` and
siblings); `tests/cli/cli_fmt_multiline_brackets.rs`.

Owner requirement (2026-10-04): "these should all parse and work. This same splitting
should work for {} records and maps as well."

## Failing Reproduction

All three must parse and produce the same 180-element list (owner's example,
abbreviated here):

```
LET testA AS List OF String = ["AED", "AFN", "ALL", …, "ZWG"]
LET testB AS List OF String = ["AED", "AFN", "ALL", "AMD",
"AOA", "ARS", …,
"ZWG"]
LET testC AS List OF String = [
  "AED", "AFN", "ALL", …,
  "XPT", "XSU", "ZWG"
]
```

Measured with the compiler at bug-staging-9 (`/tmp/b821` probe, one form per build):

| Form | One line | Break after a comma | Break after `[`/`{` and before `]`/`}` |
|---|---|---|---|
| List `[...]` | parses | 5 parse errors | 7 parse errors |
| Map `Map OF K TO V { k := v, … }` | parses | 5 parse errors | 10 parse errors |
| Record `Pt[1, 2]` | parses | 5 parse errors | 7 parse errors |
| Call arguments `f(a +⏎ b)` | parses | 2 parse errors | — |

The errors are `MFB_PARSE_EXPECTED_EXPRESSION`, `MFB_PARSE_UNEXPECTED_TOKEN` and
`MFB_PARSE_UNEXPECTED_STATEMENT` on the continuation lines.

## Root Cause

This is a language rule, not a parser defect. `mfb spec language
lexical-structure`: "Statement separator: newline (a Newline token) … Line
continuation: a trailing _ followed only by whitespace and then a newline joins
the next line". Inside an open `[`, `{` or `(` the lexer still emits Newline
tokens, so the literal ends at the line break.

## Goal

A newline inside an unclosed `[ ]`, `{ }` or `( )` is not a statement separator.
The lexer or parser suppresses Newline tokens while bracket depth > 0, like implicit
line joining in Python. Each of these spans lines, breaking after any element or
right after the opening bracket / before the closing one:
- list literals;
- map literals (`Map OF K TO V { … }`);
- set literals;
- record constructors, both the positional `Type[ … ]` form and every `{ … }`
  record form;
- `WITH r { … }` updates.

The values are identical to the one-line forms. `_` continuation keeps working.
A newline outside brackets is still a separator.

## Owner decisions (2026-10-04)

1. **Parentheses too:** call arguments and parenthesized expressions span lines
   under the same rule as `[ ]` and `{ }`.
2. **Trailing comma allowed** before any closing bracket (`"ZWG",⏎]`). A
   **leading-comma** layout (`⏎, "B"`) must also parse. `mfb fmt` rewrites
   leading commas into trailing commas.
3. **End-of-line `'` comments** inside an open bracket are allowed and ignored.

## Blast Radius

- Lexer/parser (`MFB_PARSE_*` behavior), `mfb spec language lexical-structure`
  and the collections/records topics.
- `mfb fmt` (spec `tooling/05_fmt.md`): it must preserve, or normalize, multi-line
  literals and keep its block-depth handling right. Today it may join or reject
  them.
- Diagnostics: an unclosed bracket at end of file must report at the opening
  bracket, not at some later line.
- No codegen change; `.ast`/`.ir` of existing fixtures should be unchanged.

## Phases

- [x] RED fixture with every form, one line vs split, values compared at runtime.
- [x] Bracket-depth newline suppression; unclosed-bracket diagnostic at the opener.
- [x] `mfb fmt` handling; spec and man updates.

## Resolution (2026-10-04)

**The rule (lexer).** `Lexer` keeps a stack of unclosed `(`/`[`/`{`
(`open_brackets`, `OpenBracket`). `lex_newline` emits a `Newline` only when the
stack is empty; inside a bracket the newline is skipped like whitespace. A closer
pops the innermost opener whatever its kind (a mismatched or stray closer stays
the parser's error). Strings, scalars and `'`/`REM` comments are whole tokens or
skipped text, so a bracket inside them never moves the depth. `_` continuation is
untouched. Because the token stream of a bracket written across lines is the
stream of the one-line form, the AST/IR/values are identical — the fixture
checks every form at runtime against its one-line twin (`count=180 first=AED
last=ZWG`, 16 `ok` lines).

**Trailing commas (parser).** Every comma-separated bracketed list accepts one
trailing comma before its closer: list/map/set literals, call and constructor
arguments, `WITH` updates, parameter lists (`FUNC`, `LAMBDA`), function-type
parameter lists and `LINK` ABI slot lists. A leading-comma layout needs no parser
change (the newline is gone). `[,]`, `[, 1]` and `[1,, 2]` stay parse errors
(`bracket-comma-invalid`; diagnostics identical to the parent compiler).

**Diagnostic.** `MFB_LEX_UNCLOSED_BRACKET` (`1-101-0006`), at the OPENING
bracket, (1) at end of file, and (2) when a line inside an open bracket begins
with a statement-only keyword (`lexer::begins_statement_only`: `IF`, `FOR`,
`END`, `LET`, `RETURN`, `MATCH`, … — never `FUNC`/`ISOLATED`/`MUT`/`RES`/`WITH`/
`LAMBDA`/`NOT`/operators/literals/`AS`/`TO`/`IN`, which can start a line inside
a bracket). Case 2 clears the stack and resumes normal separation, so an
unbalanced bracket never swallows the rest of the file; each unclosed bracket in
the file is reported.

**`mfb fmt`.** A physical line ending inside an open bracket continues the
logical line (same depth rule, same `begins_statement_only` stop), so
continuation lines keep their own leading whitespace exactly like `_`
continuations; nothing is joined or re-indented. A leading comma inside a bracket
moves to just after the code of the previous code line (before its comment or
`_`) and becomes a space, so the element keeps its column; a comma-only line is
dropped. A continuation line no longer counts as a statement start, so a
line-leading `REM` there is an identifier, as the lexer reads it (this also
closes the same fmt/lexer disagreement for `_` continuation lines).

**Reach check** (parent `bug-staging-11` vs fixed, `mfb build -q -ast -ir` over
all 1636 `project.json` under `tests/` and `examples/`, plus `mfb fmt` over all
1758 `.mfb` copies): the only build log/AST/IR change is
`tests/syntax/lexical/bug89_parser_eof_open_delim` (its three parse errors become
one `MFB_LEX_UNCLOSED_BRACKET` at the `(` — the behavior this bug requires; the
bug-89 Eof guard stays covered by `ast::expr::tests::primary_eof_after_trailing_operator`);
zero formatting changes; `mfb fmt --check` on the fixed output passes for every
file (idempotent).

**Expected golden move (not regenerated in this change):**
`tests/syntax/lexical/bug89_parser_eof_open_delim/golden/build.log`.

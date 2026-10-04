# bug-821: a list, map or record literal cannot span lines without `_`

Last updated: 2026-10-04
Effort: medium (guess): lexer/parser, spec, `mfb fmt`
Severity: MEDIUM: long literals (tables, code lists) are unwritable without `_` on every line
Class: Language (surface syntax)

Status: Open
Regression Test: proposed `tests/syntax/lexical/literal-spans-lines-valid` (every form
below), plus runtime checks that the values are right.

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

- [ ] RED fixture with every form, one line vs split, values compared at runtime.
- [ ] Bracket-depth newline suppression; unclosed-bracket diagnostic at the opener.
- [ ] `mfb fmt` handling; spec and man updates.

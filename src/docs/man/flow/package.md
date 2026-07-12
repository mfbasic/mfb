# flow

Control-flow statements and block forms

## Synopsis

```
mfb man flow [topic]
```

## Imports

`flow` is a documentation topic, not an importable package. Every control-flow
form is a language keyword, so no `IMPORT` is needed.

## Description

MFBASIC control flow is statement-based and structured — there is no `GOTO` and
no `SELECT CASE` (use `MATCH`). Conditions must be `Boolean`; there is no
truthiness coercion from numbers, strings, or collections. Blocks either use a
dedicated terminator such as `END IF`, `WEND`, or `LOOP UNTIL`, or a single-line
form whose statements are separated with `:`.

`EXIT FOR`, `EXIT DO`, and `EXIT WHILE` leave the innermost enclosing loop of the
matching kind; `CONTINUE FOR`, `CONTINUE DO`, and `CONTINUE WHILE` skip to its
next iteration. `EXIT SUB` leaves the enclosing `SUB` successfully, and
`EXIT PROGRAM <code>` terminates the process after running lexical cleanup for
every live scope. These are structured, lexically scoped exits, not arbitrary
jumps.

## Topics

- `if` — conditional execution with `THEN`, `ELSEIF`, `ELSE`, and `END IF`.
- `for` — counted loops with `TO`, optional `STEP`, and `NEXT`.
- `forEach` — collection iteration over `List` and `Map` values.
- `while` — pre-test loops with `WHILE` and `WEND`.
- `do` — pre-test and post-test `DO` loop forms.
- `match` — value-based branching over unions, enums, and literals.

## Errors

No errors.

## See also

- `mfb man flow if`
- `mfb man flow match`
- `mfb man types logical`
- `mfb man types comparisons`
- `mfb man errors`

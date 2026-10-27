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
truthiness coercion from numbers, strings, or collections. Every block form ends
with its terminator (`END IF`, `NEXT`, `END WHILE`, `LOOP`). Any block can be
written on one line by separating its statements with `:`, terminator included.
The one exception is single-line `IF ... THEN statement [ELSE statement]`, which
takes exactly one statement per branch and no `END IF`; a `:` after it starts a
new statement outside the `IF`. The pipeline operator `|>` is an expression
form, not a statement; it is documented here because it chains the calls a flow
statement would otherwise nest (`mfb man flow pipeline`).

`EXIT FOR`, `EXIT DO`, and `EXIT WHILE` leave the innermost enclosing loop of the
matching kind; `CONTINUE FOR`, `CONTINUE DO`, and `CONTINUE WHILE` skip to its
next iteration. `EXIT FOR name` and `CONTINUE FOR name` target the `FOR` or
`FOR EACH` loop whose variable is `name`. `os::exit(code)` terminates the process after running lexical cleanup for
every live scope; no `TRAP` catches it. Run from a worker thread, it ends the whole program, not
just that worker. These are structured, lexically scoped exits, not arbitrary jumps.

## Topics

- `if` — conditional execution with `THEN`, `ELSEIF`, `ELSE`, and `END IF`.
- `for` — counted loops with `TO`, optional `STEP`, and `NEXT`.
- `forEach` — collection iteration over `List`, `Set`, and `Map` values.
- `while` — pre-test loops with `WHILE` and `END WHILE`.
- `do` — pre-test and post-test `DO` loop forms.
- `match` — value-based branching over unions, enums, and literals.
- `pipeline` — thread a value through a chain of calls with `|>` and `_`.

## Errors

No errors.

## See also

- `mfb man flow if`
- `mfb man flow match`
- `mfb man types logical`
- `mfb man types comparisons`
- `mfb man errors`

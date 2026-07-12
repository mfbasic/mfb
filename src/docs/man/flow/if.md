# if

Conditional execution

## Synopsis

```
IF condition THEN statement
IF condition THEN statement ELSE statement
IF condition THEN
  ...
ELSEIF otherCondition THEN
  ...
ELSE
  ...
END IF
```

## Description

`IF` evaluates a `Boolean` condition and executes the matching branch. The
condition must be `Boolean` — there is no truthiness coercion from other types.

The single-line form runs one statement when the condition is `TRUE`, with an
optional single-line `ELSE`. The block form may include zero or more `ELSEIF`
clauses and an optional `ELSE`; the first branch whose condition is `TRUE` runs,
and when none match and there is no `ELSE`, no branch runs.

## Errors

No errors.

## Examples

Single-line form:

```
IF x > 0 THEN io::print("pos") ELSE io::print("non-pos")
```

Block form with `ELSEIF` and `ELSE`:

```
IF score > 90 THEN
  io::print("A")
ELSEIF score > 80 THEN
  io::print("B")
ELSE
  io::print("C")
END IF
```

## See also

- `mfb man flow match`
- `mfb man flow while`
- `mfb man types logical`

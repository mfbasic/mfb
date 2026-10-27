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
optional single-line `ELSE`. It takes exactly one statement per branch and no
`END IF`; a `:` after it starts a new statement outside the `IF`. The block form
may include zero or more `ELSEIF` clauses and an optional `ELSE`. The keyword is
the single word `ELSEIF`; `ELSE IF` on one line is a parse error, because `ELSE`
must end its line. In the block form, the first branch whose condition is `TRUE` runs,
and when none match and there is no `ELSE`, no branch runs.

## Errors

No errors.

## Examples

Single-line form:

```
IMPORT io

FUNC main() AS Nothing
  LET x AS Integer = 1
  IF x > 0 THEN io::print("pos") ELSE io::print("non-pos")
END FUNC
```

Block form with `ELSEIF` and `ELSE`:

```
IMPORT io

FUNC main() AS Nothing
  LET score AS Integer = 85
  IF score > 90 THEN
    io::print("A")
  ELSEIF score > 80 THEN
    io::print("B")
  ELSE
    io::print("C")
  END IF
END FUNC
```

## See also

- `mfb man flow match`
- `mfb man flow while`
- `mfb man types logical`

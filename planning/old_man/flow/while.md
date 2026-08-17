# while

Pre-test loops

## Synopsis

```
WHILE condition : ... : END WHILE
WHILE condition
  ...
END WHILE
```

## Description

`WHILE` evaluates its `Boolean` condition before each iteration. The condition
must be `Boolean`. When the condition is `FALSE` before the first check, the body
does not run. `EXIT WHILE` leaves the loop and `CONTINUE WHILE` skips to the next
iteration.

## Errors

No errors.

## Examples

Count to ten:

```
SUB main()
  MUT x AS Integer = 0
  WHILE x < 10 : x = x + 1 : END WHILE
END SUB
```

## See also

- `mfb man flow do`
- `mfb man flow if`

# for

Counted loops

## Synopsis

```
FOR name = start TO end : ... : NEXT
FOR name = start TO end STEP step
  ...
NEXT
```

## Description

`FOR` iterates a numeric loop variable from `start` toward `end`. The loop
variable takes the numeric type promoted across the `start`, `end`, and `step`
operands, so it may be `Byte`, `Integer`, `Fixed`, or `Float`; non-numeric
operands are a compile error (`TYPE_FOR_REQUIRES_NUMERIC`).

`STEP` is optional and defaults to 1. A positive `STEP` continues while the loop
variable is `<= end`; a negative `STEP` continues while it is `>= end`. A
constant `STEP` of zero is a compile error (`TYPE_FOR_STEP_ZERO`).

`EXIT FOR` leaves the innermost enclosing `FOR` loop and `CONTINUE FOR` skips to
its next iteration.

## Errors

No errors.

## Examples

Count up by two:

```
IMPORT io

SUB main()
  FOR i = 1 TO 5 STEP 2 : io::print(toString(i)) : NEXT
END SUB
```

Count down by two:

```
IMPORT io

SUB main()
  FOR down = 5 TO 1 STEP -2 : io::print(toString(down)) : NEXT
END SUB
```

## See also

- `mfb man flow forEach`
- `mfb man flow while`

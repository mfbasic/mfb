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
constant `STEP` of zero is a compile error (`TYPE_FOR_STEP_ZERO`). A `STEP` that
is not a constant is not checked: if it is zero when the loop starts and the loop
runs at all, the loop variable never advances, and the loop repeats until
something inside it leaves with `EXIT FOR`.

A Float counter can also stop advancing when adding a small step rounds back
to the same value. There is no progress check: for example, starting at
`9007199254740992.0` with step `1.0` repeats the same value while the bound still
holds. Use an Integer counter when you need a known number of iterations.

The increment happens after the body, before testing the bound again. A loop
from the maximum Integer to that same value executes the body once and then
raises `ErrOverflow` while incrementing. `EXIT FOR` in the body avoids that
increment; `CONTINUE FOR` does not.

`EXIT FOR` leaves the innermost enclosing `FOR` loop and `CONTINUE FOR` skips to
its next iteration.

## Errors

Evaluating or converting the start, end, or step can raise a numeric error.
The counter increment uses checked arithmetic: overflow raises `ErrOverflow`
for Integer/Fixed or an out-of-range Byte result. A Float increment producing
infinity raises `ErrFloatOverflow`; a NaN raises `ErrFloatNaN`. Finite
non-progress and a runtime zero step do not themselves raise an error.

## Examples

Count up by two:

```
IMPORT io

FUNC main() AS Nothing
  FOR i = 1 TO 5 STEP 2 : io::print(toString(i)) : NEXT
END FUNC
```

Count down by two:

```
IMPORT io

FUNC main() AS Nothing
  FOR down = 5 TO 1 STEP -2 : io::print(toString(down)) : NEXT
END FUNC
```

## See also

- `mfb man flow forEach`
- `mfb man flow while`

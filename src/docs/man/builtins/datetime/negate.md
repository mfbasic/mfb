# negate

Return a `Duration` with the opposite sign — the additive inverse of a span.

## Synopsis

```
datetime::negate(d AS Duration) AS Duration
```

## Package

datetime

## Imports

```
IMPORT datetime
```

`datetime` is a built-in package, so no manifest dependency is required.
[[src/builtins/datetime.rs:augmented_project]]

## Description

`datetime::negate` returns the additive inverse of `d`: the span of equal
magnitude that points the opposite way along a timeline. A forward span of `+90s`
becomes a backward span of `-90s`, a backward span becomes forward, and the zero
`Duration` negates to itself. Adding `d` to `datetime::negate(d)` yields a zero
span. [[src/builtins/datetime_package.mfb:__datetime_negate]]

Negation acts on the whole span, not on each field independently. It negates both
the `seconds` and the `nanos` field, then re-normalizes so the stored `nanos`
always lands in the range `0 .. 999_999_999`, carrying any borrow into the
`seconds` field. So a `Duration` whose `seconds` is `0` and whose `nanos` is
`250_000_000` (a quarter second forward) negates to a `Duration` whose `seconds`
is `-1` and whose `nanos` is `750_000_000` — the same magnitude pointing
backward. [[src/builtins/datetime_package.mfb:__datetime_normDuration]]

Negation is the same operation as `datetime::minus(zero, d)`. The arithmetic is
ordinary signed `Integer` arithmetic, so negating the most negative representable
`seconds` count has no positive counterpart in the `Integer` range and traps.
`negate` is pure: the same `Duration` always negates to the same result, and it
has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `d` | `Duration` | The span to negate. May be a forward span (positive `seconds`), a backward span (negative `seconds`), or the zero span. Its `nanos` field is assumed to be a normalized sub-second remainder in `0 .. 999_999_999`, as produced by every `datetime` constructor. [[src/builtins/datetime.rs:NEGATE]] |

## Return value

| Type | Description |
| --- | --- |
| `Duration` | The span of equal magnitude pointing the opposite way. The `seconds` field holds the negated, re-normalized whole-second count and the `nanos` field holds the sub-second remainder in `0 .. 999_999_999`. The zero span returns unchanged. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | Negating the `seconds` field produces a value outside the signed `Integer` range, which happens exactly when `seconds` is the most negative representable `Integer` (its positive counterpart is unrepresentable). [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Negate a forward span to get the matching backward span:

```
IMPORT datetime

SUB main()
  LET forward AS Duration = datetime::duration(90)
  LET backward AS Duration = datetime::negate(forward)
END SUB
```

Negation re-normalizes a sub-second span:

```
IMPORT datetime

SUB main()
  LET quarter AS Duration = datetime::duration(0, 250_000_000)
  LET back AS Duration = datetime::negate(quarter)
END SUB
```

## See also

- `mfb man datetime duration`
- `mfb man datetime plus`
- `mfb man datetime minus`
- `mfb man datetime between`
- `mfb man datetime formatDuration`

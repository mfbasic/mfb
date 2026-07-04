# equals

Test whether two instants name the same point on the UTC timeline.

## Synopsis

```
datetime::equals(a AS Instant, b AS Instant) AS Boolean
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

`datetime::equals` is a convenience predicate over instants that returns `TRUE`
when `a` and `b` name the same point on the UTC timeline and `FALSE` otherwise.
It is defined directly in terms of `datetime::compare`: the result is exactly
`datetime::compare(a, b) = 0`, so it is `TRUE` only when `compare` reports `0`
and `FALSE` when `compare` reports `-1` or `1`.
[[src/builtins/datetime_package.mfb:__datetime_equals]]

The comparison is performed field by field, matching `datetime::compare`. The
`seconds` fields are compared first; only when they are equal are the `nanos`
fields used as a tiebreaker. Two instants are equal only when both their
`seconds` and their `nanos` fields are equal, so equality is exact to the
nanosecond and there is no tolerance window. Because both arguments are points
on the same Unix-epoch, leap-second-free UTC timeline, the test is absolute and
independent of any time zone; resolve a `DateTime` to an `Instant` with
`datetime::resolve` before comparing.

`equals` is pure: the same two instants always yield the same `Boolean`, it has
no side effects, and it performs only signed comparisons (no arithmetic), so it
cannot overflow or trap. For the strict ordering tests use `datetime::isBefore`
and `datetime::isAfter`, and for a three-way sign rather than a `Boolean` use
`datetime::compare`. To measure the size of the gap between two instants rather
than just whether they coincide, use `datetime::between`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Instant` | The left operand, a point on the UTC timeline. Its `seconds` field is whole seconds since `1970-01-01T00:00:00Z` (possibly negative) and its `nanos` field is the sub-second remainder. The result is `TRUE` only when `a` names the same point as `b`. [[src/builtins/datetime.rs:EQUALS]] |
| `b` | `Instant` | The right operand, compared against `a`. A point on the UTC timeline. The result is `TRUE` only when `b` names the same point as `a`. |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `a` and `b` name the same instant, and `FALSE` otherwise. The `seconds` fields are compared first and the `nanos` fields break ties, so a `TRUE` result requires both fields to match exactly. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Equal instants compare as equal:

```
IMPORT datetime

LET a AS Instant = datetime::instant(1_000)
LET b AS Instant = datetime::instant(1_000)
PRINT datetime::equals(a, b)
```

Different instants are not equal:

```
IMPORT datetime

LET a AS Instant = datetime::instant(1_000)
LET b AS Instant = datetime::instant(2_000)
PRINT datetime::equals(a, b)
```

Branch on whether two instants coincide:

```
IMPORT datetime

LET a AS Instant = datetime::now()
LET b AS Instant = datetime::instant(0)
IF datetime::equals(a, b) THEN PRINT "same instant"
```

## See also

- `mfb man datetime compare`
- `mfb man datetime isBefore`
- `mfb man datetime isAfter`
- `mfb man datetime between`
- `mfb man datetime resolve`

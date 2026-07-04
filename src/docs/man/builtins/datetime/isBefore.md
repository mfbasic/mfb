# isBefore

Test whether one instant strictly precedes another on the UTC timeline.

## Synopsis

```
datetime::isBefore(a AS Instant, b AS Instant) AS Boolean
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

`datetime::isBefore` is a convenience predicate over instants that returns
`TRUE` when `a` strictly precedes `b` on the UTC timeline and `FALSE` otherwise.
It is defined directly in terms of `datetime::compare`: the result is exactly
`datetime::compare(a, b) < 0`, so it is `TRUE` only when `compare` reports `-1`
and `FALSE` when `compare` reports `0` or `1`.
[[src/builtins/datetime_package.mfb:__datetime_isBefore]]

The comparison is performed field by field, matching `datetime::compare`. The
`seconds` fields are compared first; only when they are equal are the `nanos`
fields used as a tiebreaker. As a consequence, two instants that name the same
point (equal `seconds` and equal `nanos`) are not "before" each other, so
`isBefore` returns `FALSE` for equal instants — the relation is strict, not
"before or equal". Because both arguments are points on the same Unix-epoch,
leap-second-free UTC timeline, the ordering is absolute and independent of any
time zone; resolve a `DateTime` to an `Instant` with `datetime::resolve` before
comparing.

`isBefore` is pure: the same two instants always yield the same `Boolean`, it
has no side effects, and it performs only signed comparisons (no arithmetic), so
it cannot overflow or trap. For the symmetric test use `datetime::isAfter`, for
an equality test use `datetime::equals`, and for a three-way sign rather than a
`Boolean` use `datetime::compare`. To measure the size of the gap between two
instants rather than just their order, use `datetime::between`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Instant` | The left operand, a point on the UTC timeline. Its `seconds` field is whole seconds since `1970-01-01T00:00:00Z` (possibly negative) and its `nanos` field is the sub-second remainder. The result is `TRUE` only when `a` falls strictly earlier than `b`. [[src/builtins/datetime.rs:IS_BEFORE]] |
| `b` | `Instant` | The right operand, compared against `a`. A point on the UTC timeline. The result is `TRUE` only when `b` falls strictly later than `a`. |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `a` is strictly before `b`, and `FALSE` when `a` is equal to or after `b`. The `seconds` fields are compared first and the `nanos` fields break ties, so a `FALSE` result includes the case where `a` and `b` name the same instant. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

An earlier instant is before a later one:

```
IMPORT datetime

LET a AS Instant = datetime::instant(1_000)
LET b AS Instant = datetime::instant(2_000)
PRINT datetime::isBefore(a, b)
```

Equal instants are not before each other:

```
IMPORT datetime

LET a AS Instant = datetime::instant(1_000)
LET b AS Instant = datetime::instant(1_000)
PRINT datetime::isBefore(a, b)
```

Branch on chronological order:

```
IMPORT datetime

LET past AS Instant = datetime::instant(0)
LET nowInstant AS Instant = datetime::now()
IF datetime::isBefore(past, nowInstant) THEN PRINT "past is earlier"
```

## See also

- `mfb man datetime isAfter`
- `mfb man datetime equals`
- `mfb man datetime compare`
- `mfb man datetime between`
- `mfb man datetime resolve`

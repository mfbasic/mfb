# compare

Order two instants on the UTC timeline as a three-way sign.

## Synopsis

```
datetime::compare(a AS Instant, b AS Instant) AS Integer
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

`datetime::compare` returns the sign of `a - b` as a three-way ordering: `-1`
when `a` is before `b`, `0` when the two instants name the same point, and `1`
when `a` is after `b`. The result is the standard comparator value suitable for
driving a sort or a branch on ordering, and it never returns any value other
than `-1`, `0`, or `1`. [[src/builtins/datetime_package.mfb:__datetime_compare]]

The comparison is performed field by field. The `seconds` fields are compared
first: if `a.seconds` is less than `b.seconds` the result is `-1`, and if it is
greater the result is `1`. Only when the `seconds` fields are equal are the
`nanos` fields compared the same way, so the sub-second component acts as a
tiebreaker. When both `seconds` and `nanos` are equal the instants are
identical and the result is `0`. Because both arguments are points on the same
Unix-epoch, leap-second-free UTC timeline, the ordering is absolute and
independent of any time zone; resolve a `DateTime` to an `Instant` with
`datetime::resolve` before comparing.

`compare` is pure: the same two instants always yield the same `Integer`, it
has no side effects, and it performs only signed comparisons (no arithmetic),
so it cannot overflow or trap. For a `Boolean` test rather than a three-way
sign, use `datetime::isBefore`, `datetime::isAfter`, or `datetime::equals`, each
of which is defined in terms of `compare`. To measure the size of the gap
between two instants rather than just their order, use `datetime::between`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Instant` | The left operand, a point on the UTC timeline. Its `seconds` field is whole seconds since `1970-01-01T00:00:00Z` (possibly negative) and its `nanos` field is the sub-second remainder. When `a` precedes `b` the result is `-1`. [[src/builtins/datetime.rs:COMPARE]] |
| `b` | `Instant` | The right operand, compared against `a`. A point on the UTC timeline. When `b` precedes `a` the result is `1`. |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | `-1` when `a` is before `b`, `1` when `a` is after `b`, and `0` when `a` and `b` name the same instant. The `seconds` fields are compared first and the `nanos` fields break ties, so only fully equal instants return `0`. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Order two instants:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(2_000)
  io::print(toString(datetime::compare(a, b)))
END SUB
```

Equal instants compare as zero:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::instant(1_000)
  LET b AS Instant = datetime::instant(1_000)
  io::print(toString(datetime::compare(a, b)))
END SUB
```

Branch on the three-way ordering:

```
IMPORT datetime
IMPORT io

SUB main()
  LET a AS Instant = datetime::now()
  LET b AS Instant = datetime::instant(0)
  LET order AS Integer = datetime::compare(a, b)
  IF order < 0 THEN io::print("a is earlier")
END SUB
```

## See also

- `mfb man datetime isBefore`
- `mfb man datetime isAfter`
- `mfb man datetime equals`
- `mfb man datetime between`
- `mfb man datetime resolve`

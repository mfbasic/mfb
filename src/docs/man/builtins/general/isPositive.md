# isPositive

Test whether a number is strictly greater than zero.

## Synopsis

```
isPositive(value AS Integer) AS Boolean
isPositive(value AS Float) AS Boolean
isPositive(value AS Fixed) AS Boolean
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`isPositive` returns `TRUE` when `value` is strictly greater than zero and
`FALSE` otherwise. The test is the strict sign comparison `value > 0`: zero is
not positive, so `isPositive(0)` is `FALSE`, and negative values are not
positive, so `isPositive(-5)` is `FALSE` while `isPositive(5)` is `TRUE`.
[[src/target/shared/code/builder_conversions.rs:lower_numeric_filter_predicate]]

`isPositive` is overloaded for the three numeric types `Integer`, `Float`, and
`Fixed`. The argument is compared against zero in its own type with no
conversion — an `Integer` and a `Fixed` compare their whole value against `0`,
and a `Float` compares against `0.0` — so the same strict rule applies whether
`value` is `7`, `0.0001`, or a `Fixed` amount. Negative zero, which the `Float`
type permits, compares equal to zero and therefore yields `FALSE`.
[[src/target/shared/code/builder_conversions.rs:lower_numeric_filter_predicate]]

`isPositive` reads only `value`; it has no side effects and never mutates its
argument. It is an inlined built-in, so it cannot be passed as a function value
directly; wrap it in a `LAMBDA` (or a named `FUNC`) where a predicate argument
is needed. The same predicate is also exposed through the `filters` package.
[[src/builtins/general.rs:filter_predicate_type]]

## Overloads

**`isPositive(value AS Integer) AS Boolean`**

Tests an `Integer` against zero, returning `TRUE` when `value > 0`.

**`isPositive(value AS Float) AS Boolean`**

Tests a `Float` against zero, returning `TRUE` when `value > 0.0`. Negative zero
yields `FALSE`.

**`isPositive(value AS Fixed) AS Boolean`**

Tests a `Fixed` against zero, returning `TRUE` when `value` is greater than zero.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer`, `Float`, or `Fixed` | The number to test. Any value of an accepted numeric type is accepted; its value alone determines the result. |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `value` is strictly greater than zero, `FALSE` otherwise. Zero returns `FALSE`; negative values return `FALSE`. |

## Errors

No errors.

## Type checking

`isPositive` accepts a single `Integer`, `Float`, or `Fixed` argument and
returns `Boolean`. Calling it with any other type, or with a different number of
arguments, is a compile-time error. Like other `general` predicates it may be
overridden by a user- or package-defined `FUNC` of the same name for its own
value types. [[src/builtins/general.rs:resolve_call]]

## Examples

Test a literal:

```
SUB main()
  LET result AS Boolean = isPositive(7)
END SUB
```

Branch on sign:

```
IMPORT io

SUB main()
  LET balance AS Integer = 5
  IF isPositive(balance) THEN
    io::print("balance is positive")
  END IF
END SUB
```

Use it as a predicate by wrapping it in a `LAMBDA`:

```
IMPORT collections

SUB main()
  LET values AS List OF Integer = [-2, -1, 0, 1, 2]
  LET positives AS List OF Integer = collections::filter(values, LAMBDA(n AS Integer) -> isPositive(n))
END SUB
```

## See also

- `mfb man general isNegative`
- `mfb man general isZero`
- `mfb man general isNumeric`
- `mfb man filters isPositive`

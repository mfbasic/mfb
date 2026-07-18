# isZero

Test whether a number is equal to zero.

## Synopsis

```
isZero(value AS Integer) AS Boolean
isZero(value AS Float) AS Boolean
isZero(value AS Fixed) AS Boolean
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`isZero` returns `TRUE` when `value` is equal to zero and `FALSE` otherwise. The
test is the equality comparison `value = 0`: positive values are not zero, so
`isZero(5)` is `FALSE`, and negative values are not zero, so `isZero(-5)` is
`FALSE`. Only an exact zero yields `TRUE`.
[[src/target/shared/code/builder_conversions.rs:lower_numeric_filter_predicate]]

`isZero` is overloaded for the three numeric types `Integer`, `Float`, and
`Fixed`. The argument is compared against zero in its own type with no
conversion — an `Integer` and a `Fixed` compare their whole value against `0`,
and a `Float` compares against `0.0` — so the same rule applies whether `value`
is `0`, `0.0`, or a `Fixed` amount. Negative zero, which the `Float` type
permits, compares equal to zero and therefore yields `TRUE`.
[[src/target/shared/code/builder_conversions.rs:lower_numeric_filter_predicate]]

`isZero` reads only `value`; it has no side effects and never mutates its
argument. It is an inlined built-in, so it cannot be passed as a function value
directly; wrap it in a `LAMBDA` (or a named `FUNC`) where a predicate argument
is needed. The same predicate is also exposed through the `filters` package.
[[src/builtins/general.rs:filter_predicate_type]]

## Overloads

**`isZero(value AS Integer) AS Boolean`**

Tests an `Integer` against zero, returning `TRUE` when `value = 0`.

**`isZero(value AS Float) AS Boolean`**

Tests a `Float` against zero, returning `TRUE` when `value = 0.0`. Negative zero
yields `TRUE`.

**`isZero(value AS Fixed) AS Boolean`**

Tests a `Fixed` against zero, returning `TRUE` when `value` is equal to zero.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer`, `Float`, or `Fixed` | The number to test. Any value of an accepted numeric type is accepted; its value alone determines the result. |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `value` is equal to zero, `FALSE` otherwise. Positive and negative values both return `FALSE`. |

## Errors

No errors.

## Type checking

`isZero` accepts a single `Integer`, `Float`, or `Fixed` argument and returns
`Boolean`. Calling it with any other type, or with a different number of
arguments, is a compile-time error. Like other `general` predicates it may be
overridden by a user- or package-defined `FUNC` of the same name for its own
value types. [[src/builtins/general.rs:resolve_call]]

## Examples

Test a literal:

```
LET result AS Boolean = isZero(0)
```

Branch on the value:

```
IF isZero(balance) THEN
  PRINT "balance is zero"
END IF
```

Use it as a predicate by wrapping it in a `LAMBDA`:

```
LET zeros AS List OF Integer = collections::filter([-1, 0, 2, 0], LAMBDA(n AS Integer) -> isZero(n))
```

## See also

- `mfb man general isPositive`
- `mfb man general isNegative`
- `mfb man general isNumeric`
- `mfb man filters isPositive`

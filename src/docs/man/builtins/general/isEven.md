# isEven

Test whether an integer is even.

## Synopsis

```
isEven(value AS Integer) AS Boolean
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`isEven` returns `TRUE` when `value` is even and `FALSE` when it is odd. An
integer is even when it is evenly divisible by two, that is when `value MOD 2`
is `0`. [[src/builtins/general.rs:resolve_call]]

The test inspects only the low bit of `value`'s two's-complement representation
(`value AND 1`), so it is exact for the whole `Integer` range with no division.
Zero is even, so `isEven(0)` is `TRUE`. Negative integers follow the same parity
rule, so `isEven(-4)` is `TRUE` and `isEven(-3)` is `FALSE`. [[src/target/shared/code/builder_conversions.rs:lower_integer_parity_predicate]]

`isEven` reads only `value`; it has no side effects and never mutates its
argument. It is an inlined built-in, so it cannot be passed as a function value
directly; wrap it in a `LAMBDA` (or a named `FUNC`) where a predicate argument
is needed. The same
predicate is also exposed through the `filters` package. [[src/builtins/general.rs:filter_predicate_type]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The integer to test. Any `Integer` is accepted; its value alone determines the result. |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `value` is evenly divisible by two, `FALSE` otherwise. Zero returns `TRUE`; negative values are tested by the same parity rule as positive ones. |

## Errors

No errors.

## Type checking

`isEven` accepts only an `Integer` argument and returns `Boolean`. Calling it
with any other type is a compile-time error. Like other `general` predicates it
may be overridden by a user- or package-defined `FUNC` of the same name for its
own value types. [[src/builtins/general.rs:resolve_call]]

## Examples

Test a literal:

```
SUB main()
  LET result AS Boolean = isEven(4)
END SUB
```

Branch on parity:

```
IMPORT io

SUB main()
  LET count AS Integer = 4
  IF isEven(count) THEN
    io::print("count is even")
  END IF
END SUB
```

Use it as a predicate by wrapping it in a `LAMBDA`:

```
IMPORT collections

SUB main()
  LET evens AS List OF Integer = collections::filter([1, 2, 3, 4], LAMBDA(n AS Integer) -> isEven(n))
END SUB
```

## See also

- `mfb man general isOdd`
- `mfb man general isNumeric`
- `mfb man filters isEven`

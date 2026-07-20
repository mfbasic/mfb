# sum

Add up the elements of an Integer, Float, or Fixed list

## Synopsis

```
collections::sum(value AS List OF Integer) AS Integer
collections::sum(value AS List OF Float) AS Float
collections::sum(value AS List OF Fixed) AS Fixed
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

## Description

`collections::sum` walks `value` from the first element to the last and adds
each element into a running total, returning that total. It is a **native**
member: the compiler emits the accumulation loop directly rather than
instantiating an MFBASIC generic. [[src/builtins/collections.rs:is_native_member]]
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_sum]]

There are exactly **three** overloads — `List OF Integer`, `List OF Float`, and
`List OF Fixed` — and the return type always matches the element type. There is
no `List OF Byte`, no `List OF Money`, and no general "any numeric list" form:
any other element type fails to resolve at compile time, and the lowering
rejects it a second time. [[src/builtins/general.rs:resolve_sum]]
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_sum]]

The accumulator is initialized to zero of the element type and the elements are
added in list order, so an empty `value` yields `0`, `0.0`, or `0.0F`
respectively without any addition being performed.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_sum]]

`value` is neither modified nor consumed. `sum` takes no callback and has no
optional argument; it is a single-argument member.
[[src/builtins/collections.rs:arity]]

For the `Integer` and `Fixed` overloads each step is a **checked** 64-bit
addition: if the running total leaves the destination range, the addition fails
with `ErrOverflow` rather than wrapping. `Fixed` shares the `Integer` path
because it is a scaled 64-bit integer. The `Float` overload uses IEEE-754
double addition and never raises — an out-of-range total becomes `±Inf` in the
usual floating-point way.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_sum]]
[[src/target/shared/code/builder_numeric.rs:emit_checked_integer_add]]

Note a wrinkle worth knowing before writing a handler: the compiler's inline-
built-in fallibility census classifies `sum` as **infallible**, so attaching an
inline `TRAP` to a `sum` call raises the `TYPE_INLINE_TRAP_DEAD_HANDLER`
diagnostic and that handler does not receive the overflow. The overflow is still
raised at run time and still propagates out of the enclosing function, where an
ordinary function-level `TRAP` can handle it.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]
[[src/rules/table.rs:TYPE_INLINE_TRAP_DEAD_HANDLER]]

To total a list of some other element type, or to accumulate with different
rules, fold it with `collections::reduce`.

## Overloads

**`collections::sum(value AS List OF Integer) AS Integer`**

Adds the elements with checked 64-bit integer addition and returns an `Integer`.
Fails with `ErrOverflow` if the running total leaves the `Integer` range.

**`collections::sum(value AS List OF Float) AS Float`**

Adds the elements with IEEE-754 double addition and returns a `Float`. Raises
nothing; ordinary floating-point rounding, `±Inf`, and `NaN` semantics apply, and
because addition is performed strictly left to right the result depends on
element order.

**`collections::sum(value AS List OF Fixed) AS Fixed`**

Adds the elements as scaled 64-bit integers and returns a `Fixed`. Being exact
base-10 arithmetic there is no rounding, but the total can overflow, failing
with `ErrOverflow`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF Integer`, `List OF Float`, or `List OF Fixed` | The list to total. Any length is accepted, including the empty list. The element type selects the overload. Also accepted under the name `collection`. [[src/builtins/general.rs:resolve_sum]] [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer`, `Float`, or `Fixed` | The total of every element, matching the element type of `value`. For an empty list, the zero of that type. [[src/builtins/general.rs:resolve_sum]] [[src/target/shared/code/builder_collection_queries.rs:lower_collection_sum]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | The running total of a `List OF Integer` or a `List OF Fixed` leaves the destination range during a checked addition. The `Float` overload cannot raise this. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] [[src/target/shared/code/builder_numeric.rs:emit_checked_integer_add]] |

## Examples

Total a list of integers:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET total AS Integer = collections::sum([1, 2, 3])
  io::print(toString(total))
  RETURN 0
END FUNC
```

Total `Float` and `Fixed` lists:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET floats AS List OF Float = [1.25, 2.5]
  LET fixeds AS List OF Fixed = [1.25F, 2.5F]
  io::print(toString(collections::sum(floats)))
  io::print(toString(collections::sum(fixeds)))
  RETURN 0
END FUNC
```

An empty list totals to zero:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::sum(empty)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections reduce`
- `mfb man collections transform`
- `mfb man collections`
- `mfb man types numeric`

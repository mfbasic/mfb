# filters

Always-in-scope Boolean predicate helpers

## Synopsis

```
isEven(4)
isPositive(value)
isEmpty(items)
filter([1, 2, 3, 4], isEven)
```

## Imports

`filters` is always in scope. Its functions need no `IMPORT` statement and no
manifest dependency. [[src/builtins/general.rs:is_general_call]]

## Description

The `filters` package provides Boolean predicate helpers for ordinary
conditionals, `MATCH` guards, and collection filter predicates. Every function
takes exactly one argument, returns a `Boolean`, and inspects that argument
without modifying it or producing side effects. [[src/builtins/general.rs:call_param_names]]

The numeric predicates test parity and sign. `isEven` and `isOdd` accept only
`Integer` values and report whether `value MOD 2` is zero. `isPositive` and
`isNegative` accept `Integer`, `Float`, and `Fixed` values and report whether
`value` is greater than or less than zero. [[src/builtins/general.rs:resolve_call]]

The emptiness predicates `isEmpty` and `isNotEmpty` are logical negations of each
other and accept `String`, `List OF T`, and `Map OF K TO V` values. They use
exactly the same length rules as `len`: a `String` is measured by Unicode scalar
count, a `List` by item count, and a `Map` by entry count, so the empty string
`""`, a list with no items, and a map with no entries are the only empty values.
[[src/builtins/general.rs:expected_arguments]]

Accepted argument types are resolved at compile time. An argument whose type a
predicate does not support is rejected during type checking rather than at run
time. Because these predicates are inlined builtins, they cannot be passed as
function values directly; wrap one in a `FUNC` when a predicate value is needed —
`filter` and the other `collections` higher-order helpers accept such a wrapper.
[[src/builtins/general.rs:filter_predicate_type]]

## Errors

No errors.

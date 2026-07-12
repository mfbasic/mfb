# lambda

Anonymous function values, function types, and capture rules

## Synopsis

```
LAMBDA(param AS Type, ...) -> expression
```

## Imports

`lambda` is a documentation topic, not an importable package. `LAMBDA` is a
language keyword and function types are compiler-owned, so no `IMPORT` is needed.

## Description

A lambda is an anonymous function value. It is written `LAMBDA`, a parenthesized
parameter list, `->`, and a single body expression:

```
LET square = LAMBDA(n AS Integer) -> n * n
```

Each parameter must declare an `AS` type; parameter types are not inferred and
parameters cannot declare default values. The result type is inferred from the
body expression. Lambdas are ordinary values that can be bound to a `LET`, passed
as an argument, returned from a function, and stored in records and collections —
all subject to the capture rules below. A lambda cannot be marked `ISOLATED`;
only an exported top-level `FUNC` may be a thread entry point.

## Function types

The type of a lambda (or of a named function used as a value) is
`FUNC(argTypes) AS ReturnType`. A binding of function type is called like any
function:

```
FUNC applyTwice(f AS FUNC(Integer) AS Integer, x AS Integer) AS Integer
  RETURN f(f(x))
END FUNC

LET addOne AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + 1
LET result AS Integer = applyTwice(addOne, 1)
```

A function with no parameters or no result is written `FUNC() AS T` and
`FUNC(T) AS Nothing` respectively. Because a `SUB` has success type `Nothing`, a
`SUB` may be passed where a `FUNC(T) AS Nothing` value is expected.

## Capture rules

A lambda may reference bindings from the enclosing scope; these are its captures.
An ordinary closure captures a copyable `LET` binding **by value**: the closure
deep-copies the captured binding into an independent copy that outlives the
capturing scope, so it observes a frozen snapshot, never the original binding's
later mutations.

- Copyable `LET` bindings may be captured by value.
- Capturing a `MUT` binding is a compile error
  (`TYPE_LAMBDA_CAPTURE_UNSUPPORTED`) in any ordinary closure, because the
  closure would observe a frozen copy rather than the live cell. This is distinct
  from reassigning an outer `MUT` from an inner block, which is allowed because
  that scope is still live.
- Capturing a resource (`RES`) handle or any other non-copyable value is also
  `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`.

### The non-escaping callback exception

The one exception the compiler allows is a lambda passed **directly** into a
compiler-proven non-escaping callback position — today only the `action`
argument of `collections::forEach`. [[src/builtins/mod.rs:is_nonescaping_callback_arg]]
Such a lambda may borrow an outer `MUT` binding and mutate it: the binding is
loaned to the callback for the duration of the synchronous call — a borrow of the
live binding, not a copy — and is the outer binding's again once the call
returns. Use an assignment as the body (its result type is `Nothing`, matching
`FUNC(T) AS Nothing`):

```
MUT total AS Integer = 0
forEach(items, LAMBDA(x AS Integer) -> total = total + x)
' total now holds the sum
```

Mutating a captured `MUT List` or `MUT Map` works the same way, and the update is
reflected in the outer binding after the call. Capturing a resource through such
a callback is still rejected. This is an internal call-bound borrow, not a
general source-level capability: non-escaping closures are not part of the v1
source language, so there are no `NONESCAPING`, `BORROW`, or lifetime
annotations.

## What is not possible

- Capturing a `MUT` binding in any ordinary (escaping) closure — assigning it to
  a `LET`, returning it, storing it in a record or collection, sending it to a
  thread, or passing it to an unknown function. Only the proven non-escaping
  callback position above is exempt.
- Observing a captured value's later mutations through a by-value capture: the
  closure holds an independent copy.
- Capturing resources or other non-copyable values.

## Errors

No errors.

## Examples

Accumulate into an outer `MUT` through `forEach` (allowed):

```
IMPORT collections

MUT total AS Integer = 0
forEach(items, LAMBDA(x AS Integer) -> total = total + x)
```

A `MUT`-capturing lambda that escapes its scope (rejected):

```
FUNC makeCounter() AS FUNC() AS Integer
  MUT total AS Integer = 0
  RETURN LAMBDA() -> total = total + 1   ' escapes: rejected
END FUNC
```

The returned closure would outlive `total`, so the capture cannot be a call-bound
loan and is rejected as `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`.

## See also

- `mfb man collections forEach`
- `mfb man collections transform`
- `mfb man collections reduce`
- `mfb man errors`

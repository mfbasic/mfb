# lambda

Anonymous function values, function types, and capture rules

## Synopsis

```
LAMBDA(param AS Type, ...) -> expression
```

## Imports

`lambda` is a documentation topic, not an importable package. `LAMBDA` is a
language keyword and function types are built in, so no `IMPORT` is needed.

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
all subject to the capture rules below. Only a name can be called: to call a
function held in a record field or a list, bind it to a `LET` first
(`LET g = ops.f` then `g(10)`); `ops.f(10)` and `collections::get(fs, 0)()` are
parse errors. A lambda cannot be marked `ISOLATED`, so
it cannot be a thread entry point: an entry point is a top-level `ISOLATED FUNC`
(see `mfb man thread start`).

## Function types

The type of a lambda (or of a named function used as a value) is
`FUNC(argTypes) AS ReturnType`. A binding of function type is called like any
function, with two differences: every argument is positional (a `FUNC` type keeps
parameter types, not names, so `f(x := 1)` is rejected), and every argument is
required — a named `FUNC` with default parameters loses its defaults when used as
a value. That is also why a `LAMBDA` cannot declare a default:

```
FUNC applyTwice(f AS FUNC(Integer) AS Integer, x AS Integer) AS Integer
  RETURN f(f(x))
END FUNC

LET addOne AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + 1
LET result AS Integer = applyTwice(addOne, 1)
```

A function with no parameters or no result is written `FUNC() AS T` and
`FUNC(T) AS Nothing` respectively. A function you declare `AS Nothing` is a
`FUNC(T) AS Nothing` value, so you can pass it wherever one is expected.

## Capture rules

A lambda may reference bindings from the enclosing scope; these are its captures.
An ordinary closure captures a copyable `LET` binding **as a copy**: the closure
keeps its own copy, which stays usable after the capturing scope ends, so it sees
a frozen snapshot, never the original binding's later changes. `mfb man variable`
explains copies in full.

- Copyable `LET` bindings may be captured; the closure gets its own copy.
- Capturing a `MUT` binding is a compile error
  (`TYPE_LAMBDA_CAPTURE_UNSUPPORTED`) in any ordinary closure, because the
  closure would observe a frozen copy rather than the live cell. This is distinct
  from reassigning an outer `MUT` from an inner block, which is allowed because
  that scope is still live.
- The capture rules apply to local bindings — a `LET` or `MUT` declared inside a
  function. A top-level `LET` or `MUT` is not captured at all: a lambda names it
  directly, like any function does, so it reads the current value every time it
  runs, including a top-level `MUT`'s later changes. An assignment-bodied lambda
  may only assign a local binding.
- A resource (`RES`) handle may be captured, directly or inside a record or a
  `List OF RES`. The closure gets an alias of the handle, exactly as a list
  element is one — never a second open thing. If the closure outlives the block
  that opened the handle (it is returned, stored in an outer variable or list, or
  kept in a module-level variable), the handle stays open while that longer-lived
  variable is in scope and closes once when its scope ends — the same rule a list
  holding the handle follows. `mfb man variable` explains aliases.
- A thread's worker view (`ThreadWorker`) may not be captured, directly or
  inside a record, because no list may hold one either. That capture is
  `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`. A parent `Thread` handle may be captured
  like any other resource.

### The non-escaping callback exception

The one exception the compiler allows is a lambda passed **directly** into a
compiler-proven non-escaping callback position — today only the `action`
argument of `collections::forEach`.
Such a lambda may mutate an outer `MUT` binding directly: while the call runs, the
lambda changes the outer binding itself, not a copy of it, and every change is
there once the call returns. Use an assignment as the body (its result type is `Nothing`, matching
`FUNC(T) AS Nothing`):

```
MUT total AS Integer = 0
forEach(items, LAMBDA(x AS Integer) -> total = total + x)
' total now holds the sum
```

Mutating a captured `MUT List` or `MUT Map` works the same way, and the update is
reflected in the outer binding after the call. A resource captured by such a callback follows the rule above. This is a special
case of `collections::forEach`, not a general feature: you cannot mark a parameter
of your own function this way, and there is no annotation for it. If you are only
accumulating into a `MUT`, a `FOR EACH x IN items` loop does the same thing with no
capture rule involved (`mfb man flow forEach`); the `forEach` exception matters
when the callback has to be a function value.

## What is not possible

- Capturing a `MUT` binding in any ordinary (escaping) closure — assigning it to
  a `LET`, returning it, storing it in a record or collection, sending it to a
  thread, or passing it to an unknown function. Only the proven non-escaping
  callback position above is exempt.
- Observing a captured local value's later changes through the closure: the
  closure holds its own copy.
- A recursive lambda: the `LET` being declared is not in scope inside its own
  body (`LET f = LAMBDA(n AS Integer) -> f(n - 1)` is
  `SYMBOL_UNKNOWN_IDENTIFIER`). Write a named `FUNC` for recursion and use it as
  a value.
- Capturing a thread's worker view (`ThreadWorker`), directly or inside a
  record.

## Errors

No errors.

## Examples

Return a closure that keeps its own copy of a `LET` value, and call it after the
function that built it has returned:

```
IMPORT io

FUNC makeAdder(base AS Integer) AS FUNC(Integer) AS Integer
  RETURN LAMBDA(x AS Integer) -> x + base
END FUNC

FUNC main() AS Nothing
  LET addTen = makeAdder(10)
  io::print(toString(addTen(2)))
END FUNC
```

Output:

```
12
```

Accumulate into an outer `MUT` through `forEach` (allowed):

```
IMPORT collections

FUNC main() AS Nothing
  LET items AS List OF Integer = [1, 2, 3]
  MUT total AS Integer = 0
  collections::forEach(items, LAMBDA(x AS Integer) -> total = total + x)
END FUNC
```

A `MUT`-capturing lambda that escapes its scope (rejected):

```
FUNC makeCounter() AS FUNC() AS Integer
  MUT total AS Integer = 0
  RETURN LAMBDA() -> total = total + 1   ' escapes: rejected
END FUNC
```

The returned closure would be called after `total`'s scope has ended, so it could
not change the live binding, and the capture is rejected as
`TYPE_LAMBDA_CAPTURE_UNSUPPORTED`.

## See also

- `mfb man collections forEach`
- `mfb man collections transform`
- `mfb man collections reduce`
- `mfb man variable`
- `mfb man flow forEach`
- `mfb man thread start`
- `mfb man errors`

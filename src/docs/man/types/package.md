# types

The MFBASIC type system

## Synopsis

```
mfb man types [topic]
```

## Imports

`types` is a documentation topic, not an importable package. The primitive,
record, error, container, and concurrency types described here are built in
and always understood by the language, so no `IMPORT` is needed. A few package
types (such as `TermSize`) become available when their package is imported.

## Description

MFBASIC has a small set of built-in types that the language always understands.
Primitive types name scalar values. Built-in templates such as `List`, `Map`,
`Set`, `MapEntry`, `Pair`, `Partition`, `Thread`, and `ThreadWorker` always take
their element types, so every concrete use — a `List OF Integer`, a
`Map OF String TO Float` — has a fully known type when the program is compiled.

User-defined `TYPE`, `UNION`, `ENUM`, and package-scope `RESOURCE … CLOSE BY`
declarations (native `LINK` resources) create additional program types, but they
are not built-in types. Callable signatures such as `FUNC(Integer) AS String`
are first-class function type forms rather than named built-in data types.

## Primitives

The eight core primitives are scalar value types:

- **`Boolean`** — a truth value; the literals are `TRUE` and `FALSE`.
- **`Byte`** — an unsigned 8-bit integer with range 0 through 255. An `Integer`
  literal may initialize a `Byte` only when it is statically in range; runtime
  conversion uses `toByte` and fails on out-of-range values.
- **`Integer`** — a 64-bit signed integer. Integer arithmetic is checked;
  overflow is an error rather than wraparound.
- **`Float`** — a 64-bit IEEE floating-point value. MFBASIC guarantees that no
  user-accessible `Float` is non-finite: operations that would make a value NaN
  or infinite fail when the value becomes observable.
- **`Fixed`** — a deterministic 64-bit binary fixed-point number with a signed
  32/32 split and resolution 1 / 2^32, ranging approximately -2147483648.0
  through 2147483647.9999999998. It is not exact decimal arithmetic.
- **`Money`** — an exact base-10 fixed-point amount scaled to five decimal
  places, for auditable financial values. A `Money` literal carries an `m` or `M`
  suffix (`1.25m`). It is *dimensioned*: it adds and subtracts only against
  another `Money`, scales by a plain number, and compares only against another
  `Money`, so `amount = 5` is a compile error and `amount = toMoney(5)` is the
  way to write it. See `mfb man types numeric` and `mfb man money`.
- **`String`** — an immutable UTF-8 string. Length, search, and substring
  operations use zero-based Unicode scalar indexes, not byte offsets or
  grapheme-cluster indexes.
- **`Scalar`** — a single 32-bit Unicode scalar value: a code point in
  `U+0000..U+D7FF` or `U+E000..U+10FFFF` (surrogates excluded), written as a single scalar between backticks. It is one scalar,
  not a grapheme cluster. `Scalar` is non-numeric — the arithmetic operators
  reject it — so code-point math goes through `toInt` and `toScalar`. See
  `mfb man types string`.

`Nothing` is the unit type; its only value is `NOTHING`. A function declared
`AS Nothing` has success type `Nothing`.

A `Nothing` value works like any other value. You can bind it, pass it, return it,
compare it, store it in a record field, and collect it. Calling a
`FUNC … AS Nothing` gives you `NOTHING`, so `LET done = logLine("x")` is fine. A
`List OF Nothing` counts its elements. Every `NOTHING` equals every other, so a
`Set OF Nothing` holds at most one element and a `Map OF Nothing TO V` holds at
most one entry. `Nothing` is comparable but not orderable, so
`collections::sort` rejects it. A callback that returns `Nothing` cannot stand in
for one that must produce a value: `collections::transform` rejects it, and
`collections::forEach` accepts it.

```
IMPORT io
IMPORT collections

FUNC logLine(msg AS String) AS Nothing
  io::print(msg)
  RETURN NOTHING
END FUNC

FUNC main() AS Nothing
  LET done = logLine("saved")
  LET marks = [done, logLine("again"), NOTHING]
  io::print(toString(len(marks)))
  io::print(toString(len(collections::toSet(marks))))
  IF done = NOTHING THEN io::print("done")
END FUNC
```

prints:

```
saved
again
3
1
done
```

See `mfb man types numeric` for numeric literal defaults, the promotion table,
and the checked-arithmetic error rules.

## Error type

`Error` is the standard failure payload; all language-level failures use it, and
there are no per-function error types. It and its location record `ErrorLoc` are
read-only — a program may read their fields but may not construct or update them:

```
TYPE Error
  code    AS Integer
  message AS String
  source  AS ErrorLoc
END TYPE
```

Every `FUNC` implicitly either produces its value (auto-unwrapped) or
fails with an `Error`; handle a failure with `TRAP` (see `mfb man errors`).
`Result` and `Ok` are ordinary names: a program may declare its own
`TYPE Result` or `TYPE Ok` like any other type.

## Containers

- **`List OF T`** — an ordered sequence of values of type `T` with
  zero-based indexes. `LET` list values are immutable snapshots; `MUT` list
  bindings may be updated locally. See `mfb man types list`.
- **`Map OF K TO V`** — a key/value mapping. `K` must be comparable. Map
  iteration order is implementation-defined but stable for a given unchanged map
  value during one program run. See `mfb man types map`.
- **`Set OF T`** — an unordered, deduplicated collection: each distinct
  element appears at most once, and adding a present element is a no-op. `T`
  must be comparable, exactly as a `Map` key must be. See `mfb man types set`.
- **`MapEntry OF K TO V`** — the built-in record produced by `FOR EACH`
  over a map, with public read-only `key AS K` and `value AS V` fields. See
  `mfb man flow forEach`.
- **`Pair OF (A, B)`** — a built-in two-value record used by
  `collections::zip`, with fields `first AS A` and `second AS B` and no
  comparability constraint on `A` or `B`. See `mfb man types pair`.
- **`Partition OF T`** — a built-in record returned by
  `collections::partition`, with fields `matched AS List OF T` and
  `unmatched AS List OF T`. See `mfb man types partition`.

## Concurrency

- **`Thread OF Msg TO Out`** — an opaque parent-side handle to an isolated worker
  thread. `Msg` is the message type used with `thread::send`/`thread::receive`;
  `Out` is the worker entry function's success type. A `Thread` is a `RES` handle,
  like an open file: bind it with `RES`, and it closes when its scope ends — a
  thread still running then is asked to stop. It is not copyable, but it can be
  handed to another thread with `thread::transfer`, kept in a `List OF RES
  Thread …`, and given its own state with a trailing `STATE S`.
- **`ThreadWorker OF Msg TO Out`** — the opaque worker-side handle to the same
  thread, passed into the thread entry function. It is a `RES` handle too: bind
  it with `RES`, never `LET` or `MUT`. It belongs to the worker for as long as the
  worker runs — it stays open until the worker function returns — so it never
  goes into a collection or to another thread.

## Package types

- **`color::Color`** — the colour type every package speaks, returned by
  `term::getForeground`/`term::getBackground` and taken by
  `term::setForeground`/`term::setBackground`. Unlike the other entries here it is
  an **ordinary value record**, not a built-in read-only one: a program may
  build one and `WITH`-update it. A terminal has no alpha channel, so the getters
  always report `alpha` `255` and the setters ignore it.
- **`TermSize`** — the `term` record returned by `term::terminalSize`, with
  `columns` and `rows` (`Integer`) fields for terminal width and height in
  character cells.

## Defining types

A program adds its own types with declarations. `TYPE` declares a record of named
fields, built with square brackets; `UNION` groups existing record types into one
closed choice, taken apart with `MATCH`; `ENUM` names a fixed set of members,
written `Color.Green`:

```
IMPORT io

TYPE Point
  x AS Integer
  y AS Integer
END TYPE

TYPE Circle
  center AS Point
  radius AS Integer
END TYPE

TYPE Square
  corner AS Point
  side   AS Integer
END TYPE

UNION Shape
  Circle
  Square
END UNION

ENUM Color
  Red, Green, Blue
END ENUM

FUNC main() AS Nothing
  LET origin = Point[x := 0, y := 0]
  LET s AS Shape = Square[corner := origin, side := 3]
  LET c = Color.Green
  MATCH s
    CASE Circle(ci)
      io::print("circle " & toString(ci.radius))
    CASE Square(sq)
      io::print("square " & toString(sq.side))
  END MATCH
  IF c = Color.Green THEN io::print("green")
END FUNC
```

Output:

```
square 3
green
```

A constructor's arguments can span lines, like a list literal's items
(`mfb man types list`):

```
LET corner = Point[
  x := 10,
  y := 20,
]
```

A constructor that writes its fields by name may leave some out; each one it leaves
out starts at its default (`0`, `FALSE`, `""`, the first enum member, an empty
list, map or set), the same value `MUT p AS Point` gives. `Point[x := 5]` is a
`Point` whose `y` is `0`, and `Point[]` is the all-defaults `Point`. A field with
no default (a function, a resource handle, a union) must be given. A constructor
that lists its values in field order, `Point[5, 6]`, still needs all of them.

`WITH` builds a changed copy of a record (`mfb man variable`). `RESOURCE … CLOSE BY`
declares a handle type for a native library binding (`mfb man link`).

## Comparability and copying

Comparable types (`=`, `<>`) are `Integer`, `Float`, `Fixed`, `Money`,
`Boolean`, `String`, `Byte`, `Scalar`, `Nothing`, enum types, the built-in
`Error`/`ErrorLoc` records, and records whose fields are all comparable.
Orderable types (`<`, `>`, `<=`, `>=`) are the narrower set `Integer`, `Float`,
`Fixed`, `Money`, `Byte`, `String`, and `Scalar`. Two of those compare and order
only against their own type: a `Money` never against a bare number, a `Scalar`
never against a number or a `String`. `List`, `Map`, `Set`, unions, functions,
lambdas, threads, and resource handles are neither comparable nor orderable. Map
keys and list search helpers require comparable types; `collections::sort`
requires orderable ones. See `mfb man types comparisons`.

Primitives, `String`, enums, `Nothing`, records whose fields are copyable, and
unions whose active payload is copyable are copyable. `List`, `Map`, and `Set`
are copyable only when their element, key, and value types are copyable; copying
a collection copies its contents. Thread and resource handles are not copyable. A
record with a `RES` field can still be assigned: its other fields are copied, and
the field stays an alias of the same open handle, so closing it through either
record closes it for both (see `mfb man variable`).

## Errors

No errors.

## See also

- `mfb man types comparisons`
- `mfb man types list`
- `mfb man types logical`
- `mfb man types map`
- `mfb man types numeric`
- `mfb man types pair`
- `mfb man types partition`
- `mfb man types set`
- `mfb man types string`
- `mfb man flow forEach`
- `mfb man flow match`
- `mfb man variable`
- `mfb man link`
- `mfb man errors`
- `mfb man general`
- `mfb man thread`

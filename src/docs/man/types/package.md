# types

The MFBASIC type system

## Synopsis

```
mfb man types [topic]
```

## Imports

`types` is a documentation topic, not an importable package. The primitive,
record, error, container, and concurrency types described here are compiler-owned
and always understood by the language, so no `IMPORT` is needed. A few package
types (such as `TermColor`) become available when their package is imported.

## Description

MFBASIC has a small set of compiler-owned types that the language always
understands. Primitive types name scalar values. Compiler-owned templates such
as `List`, `Map`, `MapEntry`, `Pair`, `Partition`, `Thread`, and `ThreadWorker`
are monomorphized before code is generated, so each concrete use has a fully
known type.

User-defined `TYPE`, `UNION`, `ENUM`, and package-scope `RESOURCE … CLOSE BY`
declarations (native `LINK` resources) create additional program types, but they
are not built-in types. Callable signatures such as `FUNC(Integer) AS String`
are first-class function type forms rather than named built-in data types.

## Primitives

The six core primitives are scalar value types:

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
- **`String`** — an immutable UTF-8 string. Length, search, and substring
  operations use zero-based Unicode scalar indexes, not byte offsets or
  grapheme-cluster indexes.

`Nothing` is the unit type; its only value is `NOTHING`. A `SUB` has success type
`Nothing`. `Money` is a built-in scalar for auditable financial amounts — an
exact base-10 fixed-point value scaled to five decimal places — with a restricted
dimensional algebra; see `mfb man types numeric` and `mfb man money`. [[src/numeric.rs:MONEY_SCALE]]

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

Every `FUNC` and `SUB` implicitly either produces its value (auto-unwrapped) or
fails with an `Error`. The private `Result`/`Ok`/`Err` representation of a
fallible outcome is internal — it cannot be named, constructed, or matched in
user code. See `mfb man errors`.

## Containers

- **`List OF T`** — an owned ordered sequence of values of type `T` with
  zero-based indexes. `LET` list values are immutable snapshots; `MUT` list
  bindings may be updated locally. See `mfb man types list`.
- **`Map OF K TO V`** — an owned key/value mapping. `K` must be comparable. Map
  iteration order is implementation-defined but stable for a given unchanged map
  value during one program run. See `mfb man types map`.
- **`MapEntry OF K TO V`** — the compiler-owned record produced by `FOR EACH`
  over a map, with public read-only `key AS K` and `value AS V` fields.
- **`Pair OF A, B`** — a compiler-owned two-value product used by
  `collections::zip`, with fields `first AS A` and `second AS B` and no
  comparability constraint on `A` or `B`. See `mfb man types pair`.
- **`Partition OF T`** — a compiler-owned record returned by
  `collections::partition`, with fields `matched AS List OF T` and
  `unmatched AS List OF T`. See `mfb man types partition`.

## Concurrency

- **`Thread OF Msg TO Out`** — an opaque parent-side handle to an isolated worker
  thread. `Msg` is the message type used with `thread::send`/`thread::receive`;
  `Out` is the worker entry function's success type. Thread handles are neither
  copyable nor sendable.
- **`ThreadWorker OF Msg TO Out`** — the opaque worker-side handle to the same
  thread, passed into the thread entry function.

## Package types

- **`TermColor`** — the `term` record returned by `term::getForeground` and
  `term::getBackground`, with `r`, `g`, and `b` (`Byte`) fields for the current
  24-bit color components. [[src/builtins/term.rs:TERM_COLOR_TYPE]]
- **`TermSize`** — the `term` record returned by `term::terminalSize`, with
  `columns` and `rows` (`Integer`) fields for terminal width and height in
  character cells. [[src/builtins/term.rs:TERM_SIZE_TYPE]]

## Comparability and ownership

Comparable types (`=`, `<>`) are `Integer`, `Float`, `Fixed`, `Boolean`,
`String`, `Byte`, `Nothing`, enum types, the built-in `Error`/`ErrorLoc` records,
and records whose fields are all comparable. Orderable types (`<`, `>`, `<=`,
`>=`) are the narrower set `Integer`, `Float`, `Fixed`, `Byte`, and `String`.
`List`, `Map`, unions, functions, lambdas, threads, and resource handles are
neither comparable nor orderable. Map keys and list search helpers require
comparable types; `collections::sort` requires orderable ones. See
`mfb man types comparisons`.

Primitives, `String`, enums, `Nothing`, records whose fields are copyable, and
unions whose active payload is copyable are copyable. `List` and `Map` are
copyable only when their element, key, and value types are copyable; copying a
collection copies its contents. Thread and resource handles are not copyable.

## Errors

No errors.

## See also

- `mfb man types numeric`
- `mfb man types list`
- `mfb man types map`
- `mfb man errors`
- `mfb man general`
- `mfb man thread`

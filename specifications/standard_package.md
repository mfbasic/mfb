# ⟪MFBASIC⟫ — Standard Package

Standard-package functions follow the language call rules in `mfbasic.md`: positional and named call arguments are both valid, named arguments bind by declared parameter name, and omitted trailing defaults are filled before lowering.

## 1. Built-in Types

These types are always in scope. They do not require `IMPORT`.

| Type | Description |
|------|-------------|
| `Integer` | 64-bit signed integer. |
| `Float` | 64-bit IEEE floating-point number. |
| `Fixed` | 64-bit binary fixed-point number with a signed 32/32 split. |
| `Boolean` | `TRUE` or `FALSE`. |
| `String` | Immutable UTF-8 string. |
| `Byte` | Unsigned 8-bit integer. |
| `Nothing` | Unit type with the single value `NOTHING`. |
| `Error` | Read-only error payload: `code AS Integer`, `message AS String`, `source AS ErrorLoc`. |
| `ErrorLoc` | Read-only source location of an error: `filename AS String`, `line AS Integer`, `char AS Integer`. |
| `MapEntry OF K TO V` | Standard map iteration entry: `MapEntry[key AS K, value AS V]`. |
| `Pair OF A, B` | Compiler-owned two-value product used by `collections::zip`: `first AS A`, `second AS B`. |
| `Partition OF T` | Compiler-owned split result of `collections::partition`: `matched AS List OF T`, `unmatched AS List OF T`. |
| `Thread OF Msg TO Out` | Opaque handle to an isolated thread with message type `Msg` and result type `Out`. |

The `Error` and `ErrorLoc` shapes are built into the language as read-only,
compiler/runtime-generated records:

```basic
TYPE ErrorLoc
  filename AS String
  line     AS Integer
  char     AS Integer
END TYPE

TYPE Error
  code    AS Integer
  message AS String
  source  AS ErrorLoc
END TYPE
```

A function call either produces its value or fails with an `Error`; the success
value auto-unwraps and the `Error` auto-propagates. Programs create errors with
the `error` built-in, read `e.code`/`e.message`/`e.source`, and bind the error in
`TRAP(e)`. Neither `Error` nor `ErrorLoc` may be constructed with `[...]`, updated
with `WITH`, or have their fields assigned. `Error.source` records where the error
originated (preserved across propagation; an imported-package error reports the
package's source). There is no user-visible wrapper type around a result: the
runtime represents a fallible outcome internally as a private two-member union (a
success member plus `Error`), which is not nameable, constructible, or matchable
in user code (see `mfbasic.md` §4.4).

```basic
' Create a user-authored error (always in scope, like toString):
FUNC error(code AS Integer, message AS String) AS Error
```

`MapEntry OF K TO V` is a compiler-owned record shape produced by iterating a `Map OF K TO V`. It has public read-only fields:

```basic
TYPE MapEntry OF K TO V
  key AS K
  value AS V
END TYPE
```

`Pair OF A, B` and `Partition OF T` are compiler-owned, always-in-scope generic records (`collections::zip` returns `List OF Pair OF A, B`; `collections::partition` returns `Partition OF T`). Unlike `MapEntry`, they are ordinary constructible records — public, copyable when their members are copyable, and thread-sendable when their members are sendable — and `Pair` places no comparability constraint on `A` or `B`. The names `Pair` and `Partition` are reserved; a user `TYPE` may not redeclare them.

```basic
TYPE Pair OF A, B
  first  AS A
  second AS B
END TYPE

TYPE Partition OF T
  matched   AS List OF T
  unmatched AS List OF T
END TYPE
```

## 2. Built-in Containers

The standard containers are owned value types. A `LET` container is an immutable snapshot. A `MUT` container is a locally mutable buffer while the binding is live. Map iteration order is implementation-defined but stable for a given unchanged map value: repeated `keys`, `values`, or `FOR EACH` traversal of the same map value in one program run must use the same order. Creating a changed map value may choose a different order.

| Container | Description |
|-----------|-------------|
| `List OF T` | Ordered sequence of values. |
| `Map OF K TO V` | Key/value mapping. Keys must be comparable. |

List literals use square brackets:

```basic
LET nums = [1, 2, 3]
LET empty AS List OF String = []
```

When a list literal has a declared or expected `List OF T` type, each item is checked against `T`; otherwise `T` is inferred from the first item.

Map literals use `Map OF K TO V { ... }`:

```basic
LET ages = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
```

## 3. Built-in Functions

These functions are always in scope unless a package-qualified form is shown. Fallible functions can fail and therefore auto-propagate unless their failure is caught by an inline or function-level `TRAP`.

### 3.1 General

String length, search, substring, and regex indexes are zero-based Unicode scalar indexes, not byte offsets and not grapheme-cluster indexes. Use `strings::graphemes` when user-perceived character clusters are needed.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `error` | `FUNC error(code AS Integer, message AS String) AS Error` | Creates a read-only `Error` whose `source` is the location of this `error(...)` call. Always in scope. |
| `len` | `FUNC len(value AS String) AS Integer` | Number of Unicode scalar values in `value`. |
| `len` | `FUNC len OF T(value AS List OF T) AS Integer` | Number of items in `value`. |
| `len` | `FUNC len OF K, V(value AS Map OF K TO V) AS Integer` | Number of entries in `value`. |
| `typeName` | `FUNC typeName OF T(value AS T) AS String` | Implementation-defined display name of the static type. Intended for diagnostics. |
| `toString` | `FUNC toString(value AS Integer) AS String` | Converts an integer to base-10 text. |
| `toString` | `FUNC toString(value AS Float, precision AS Byte = 2) AS String` | Converts a float to decimal text with exactly `precision` digits after the decimal point. |
| `toString` | `FUNC toString(value AS Fixed, precision AS Byte = 2) AS String` | Converts a fixed-point value to decimal text. |
| `toString` | `FUNC toString(value AS Boolean) AS String` | Returns `"TRUE"` or `"FALSE"`. |
| `toString` | `FUNC toString(value AS String) AS String` | Returns `value` unchanged. |
| `toString` | `FUNC toString(value AS Byte) AS String` | Converts a byte to base-10 text. |
| `toString` | `FUNC toString(value AS List OF Byte) AS String` | Decodes a UTF-8 byte list into text. Fails with `77020004` on invalid UTF-8. |
| `toInt` | `FUNC toInt(value AS String) AS Integer` | Parses a base-10 `Integer`. Fails with `77050003` on invalid input and `77050010` on overflow. |
| `toInt` | `FUNC toInt(value AS Byte) AS Integer` | Converts a `Byte` to `Integer`. |
| `toInt` | `FUNC toInt(value AS Float) AS Integer` | Converts a `Float` to `Integer` by truncating toward zero. Fails with `77050010` on overflow and `77050003` on `NaN` or infinity. |
| `toInt` | `FUNC toInt(value AS Fixed) AS Integer` | Converts a `Fixed` to `Integer` by truncating toward zero. |
| `toFloat` | `FUNC toFloat(value AS String) AS Float` | Parses a `Float`. Fails with `77050003` on invalid input and `77050010` on overflow. |
| `toFloat` | `FUNC toFloat(value AS Integer) AS Float` | Converts an `Integer` value to the nearest representable `Float`. |
| `toFloat` | `FUNC toFloat(value AS Fixed) AS Float` | Converts a `Fixed` value to the nearest representable `Float`. |
| `toFixed` | `FUNC toFixed(value AS String) AS Fixed` | Parses a decimal fixed-point value, rounded to nearest representable `Fixed`. Fails with `77050003` on invalid input and `77050010` on overflow. |
| `toFixed` | `FUNC toFixed(value AS Integer) AS Fixed` | Converts an `Integer` value to `Fixed`. Fails with `77050010` when outside the `Fixed` range. |
| `toFixed` | `FUNC toFixed(value AS Float) AS Fixed` | Converts a `Float` value to the nearest representable `Fixed`. Fails with `77050010` on overflow and `77050003` on `NaN` or infinity. |
| `toByte` | `FUNC toByte(value AS Integer) AS Byte` | Converts an integer to `Byte`. Fails with `77050010` when outside `0` through `255`. |
| `isNumeric` | `FUNC isNumeric(value AS String) AS Boolean` | `TRUE` when `value` can be parsed as an `Integer`, `Float`, or `Fixed`. |

`toString` is defined only for the overloads listed above. Calling `toString` on user-defined records, unions, enums, resources, threads, functions, or lambdas is a compile-time type error.

`typeName`, `toString`, and diagnostic messages are not security boundaries. Programs must not rely on them to redact secrets or to decide whether a value is safe to log. Secret-safe output requires explicit application-level formatting that omits or redacts sensitive fields.

### 3.2 Collections

These core sequence and map operations are exported by the built-in
`collections` package. `IMPORT collections` needs no manifest dependency, exactly
like `IMPORT math`. The `find`/`mid`/`replace`/`contains` entries here are the
**List** overloads; their **String** overloads live in the `strings` package
(§5).

| Function | Signature | Behavior |
|----------|-----------|----------|
| `collections::get` | `FUNC get OF T(value AS List OF T, index AS Integer) AS T` | Returns the item at zero-based `index`. Fails with `77050001` when out of range. |
| `collections::get` | `FUNC get OF K, V(value AS Map OF K TO V, key AS K) AS V` | Returns the value for `key`. Fails with `errorCode::ErrNotFound` (`77050004`) when missing. |
| `collections::getOr` | `FUNC getOr OF T(value AS List OF T, index AS Integer, default AS T) AS T` | Returns the indexed item or `default`. |
| `collections::getOr` | `FUNC getOr OF K, V(value AS Map OF K TO V, key AS K, default AS V) AS V` | Returns the mapped value or `default`. |
| `collections::find` | `FUNC find OF T(value AS List OF T, item AS T, start AS Integer = 0) AS Integer` | Zero-based index of the first matching item at or after `start`. `T` must be comparable. Fails with `errorCode::ErrNotFound` (`77050004`) when absent and `77050001` when `start` is out of range. |
| `collections::find` | `FUNC find OF T(value AS List OF T, needle AS List OF T, start AS Integer = 0) AS Integer` | Zero-based index of the first contiguous `needle` sublist at or after `start`. `T` must be comparable. Fails with `errorCode::ErrNotFound` (`77050004`) when absent and `77050001` when `start` is out of range. |
| `collections::mid` | `FUNC mid OF T(value AS List OF T, start AS Integer, count AS Integer) AS List OF T` | Returns a sublist by zero-based item index. Fails with `77050001` on invalid range. |
| `collections::replace` | `FUNC replace OF T(value AS List OF T, old AS T, new AS T) AS List OF T` | Returns a list where every item equal to `old` is replaced with `new`. `T` must be comparable. |
| `collections::set` | `FUNC set OF T(value AS List OF T, index AS Integer, item AS T) AS List OF T` | Returns a list with `item` at `index`. Fails with `77050001` when out of range. |
| `collections::set` | `FUNC set OF K, V(value AS Map OF K TO V, key AS K, item AS V) AS Map OF K TO V` | Returns a map with `key` set to `item`. |
| `collections::append` | `FUNC append OF T(value AS List OF T, item AS T) AS List OF T` | Returns a list with `item` added at the end. |
| `collections::append` | `FUNC append OF T(value AS List OF T, items AS List OF T) AS List OF T` | Returns a list with all `items` added at the end. |
| `collections::prepend` | `FUNC prepend OF T(value AS List OF T, item AS T) AS List OF T` | Returns a list with `item` added at the start. |
| `collections::insert` | `FUNC insert OF T(value AS List OF T, index AS Integer, item AS T) AS List OF T` | Returns a list with `item` inserted before `index`. Fails with `77050001` when out of range. |
| `collections::removeAt` | `FUNC removeAt OF T(value AS List OF T, index AS Integer) AS List OF T` | Returns a list without the item at `index`. Fails with `77050001` when out of range. |
| `collections::removeKey` | `FUNC removeKey OF K, V(value AS Map OF K TO V, key AS K) AS Map OF K TO V` | Returns a map without `key`. Missing keys are ignored. |
| `collections::keys` | `FUNC keys OF K, V(value AS Map OF K TO V) AS List OF K` | Returns the keys in implementation-defined stable order. |
| `collections::values` | `FUNC values OF K, V(value AS Map OF K TO V) AS List OF V` | Returns the values in key iteration order. |
| `collections::hasKey` | `FUNC hasKey OF K, V(value AS Map OF K TO V, key AS K) AS Boolean` | `TRUE` when `key` exists. |
| `collections::contains` | `FUNC contains OF T(value AS List OF T, item AS T) AS Boolean` | `TRUE` when `item` appears in the list. `T` must be comparable. |
| `collections::forEach` | `FUNC forEach OF T(value AS List OF T, action AS FUNC(T) AS Nothing) AS Nothing` | Calls `action` once for each item, left to right. A `SUB(T)` is accepted for `action`. |
| `collections::transform` | `FUNC transform OF T, U(value AS List OF T, f AS FUNC(T) AS U) AS List OF U` | Maps each item through `f`. |
| `collections::filter` | `FUNC filter OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS List OF T` | Keeps items where `predicate` returns `TRUE`. |
| `collections::reduce` | `FUNC reduce OF T, U(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U` | Folds items left to right. |
| `collections::sum` | `FUNC sum(value AS List OF Integer) AS Integer` | Sums integers. |
| `collections::sum` | `FUNC sum(value AS List OF Float) AS Float` | Sums floats. |
| `collections::sum` | `FUNC sum(value AS List OF Fixed) AS Fixed` | Sums fixed-point values. Fails with `77050010` on overflow. |

Collection callback parameters accept named functions, `SUB` values where `FUNC(... ) AS Nothing` is expected, and lambdas or closures that satisfy the language closure rules. Ordinary closures may capture only copyable `LET` bindings by value; capturing `MUT`, resource, or other non-copyable values is a compile-time error.

Ordinary `List` and `Map` values do not accept element, key, or value types that directly or transitively contain a resource handle or `Thread` handle. Ownership analysis rejects those collection instantiations before lowering.

When absence is expected, handle `collections::find` with an inline `TRAP`:

```basic
IMPORT collections
IMPORT errorCode

LET separator = collections::find(parts, "=") TRAP(e)
  IF e.code = errorCode::ErrNotFound THEN RECOVER -1   ' absent: use a sentinel
  FAIL e                                               ' any other error: bail
END TRAP
IF separator >= 0 THEN
  io::print("separator at " & toString(separator))
ELSE
  io::print("separator not found")
END IF
```

### 3.3 Collections Package — higher-level helpers

The following higher-level sequence and map helpers are exported by the same
built-in `collections` package as §3.2. `IMPORT collections` needs no manifest
dependency, like `IMPORT math`.
Element and key types follow the comparable/orderable rules (`mfbasic.md`
§4.11): `collections::sort`/`collections::sortBy` require an orderable element or
key type; `collections::distinct` requires a comparable element type.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `collections::sort` | `FUNC sort OF T(value AS List OF T) AS List OF T` | Ascending, stable sort. `T` must be orderable. |
| `collections::sortBy` | `FUNC sortBy OF T, U(value AS List OF T, keyFn AS FUNC(T) AS U) AS List OF T` | Ascending stable sort by `keyFn(item)`. `U` must be orderable. |
| `collections::take` | `FUNC take OF T(value AS List OF T, count AS Integer) AS List OF T` | First `count` items. Clamps: `count >= len` → whole list, `count <= 0` → `[]`. Total. |
| `collections::drop` | `FUNC drop OF T(value AS List OF T, count AS Integer) AS List OF T` | All but the first `count` items. Clamps: `count >= len` → `[]`, `count <= 0` → whole list. Total. |
| `collections::reduceRight` | `FUNC reduceRight OF T, U(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U` | Folds right to left. |
| `collections::any` | `FUNC any OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean` | `TRUE` when `predicate` holds for at least one item. Empty list → `FALSE`. |
| `collections::all` | `FUNC all OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean` | `TRUE` when `predicate` holds for every item. Empty list → `TRUE`. |
| `collections::findIndex` | `FUNC findIndex OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean, start AS Integer = 0) AS Integer` | Zero-based index of the first item at or after `start` satisfying `predicate`. Fails `ErrNotFound` (`77050004`) when none, `77050001` when `start` out of range. |
| `collections::findLastIndex` | `FUNC findLastIndex OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean, end AS Integer = -1) AS Integer` | Zero-based index of the last item at or before `end` (negative indexes from the end) satisfying `predicate`. Fails `ErrNotFound` when none, `77050001` when `end` out of range. |
| `collections::groupBy` | `FUNC groupBy OF T, K, V(value AS List OF T, keyFn AS FUNC(T) AS K, valFn AS FUNC(T) AS V) AS Map OF K TO List OF V` | Groups items by `keyFn`, mapping each through `valFn`. `K` comparable. Group lists preserve original order. |
| `collections::mapValues` | `FUNC mapValues OF K, V, U(value AS Map OF K TO V, f AS FUNC(V) AS U) AS Map OF K TO U` | Maps each value through `f`, keys unchanged. |
| `collections::flatten` | `FUNC flatten OF T(value AS List OF List OF T) AS List OF T` | Concatenates the inner lists in order. |
| `collections::zip` | `FUNC zip OF A, B(a AS List OF A, b AS List OF B) AS List OF Pair OF A, B` | Pairs items position-wise. Stops at the shorter input. Uses `Pair`. |
| `collections::chunks` | `FUNC chunks OF T(value AS List OF T, chunkSize AS Integer) AS List OF List OF T` | Consecutive chunks of `chunkSize`; the final chunk may be shorter. Empty input → `[]`. Fails `77050002` when `chunkSize < 1`. |
| `collections::window` | `FUNC window OF T(value AS List OF T, size AS Integer, step AS Integer = 1) AS List OF List OF T` | Sliding windows of `size`, advancing by `step`. No window when `size > len`. Fails `77050002` when `size < 1` or `step < 1`. |
| `collections::distinct` | `FUNC distinct OF T(value AS List OF T) AS List OF T` | Removes duplicates, keeping first-occurrence order. `T` comparable. |
| `collections::merge` | `FUNC merge OF K, V(a AS Map OF K TO V, b AS Map OF K TO V, preferB AS Boolean) AS Map OF K TO V` | Union of two maps. On key collision, `b` wins when `preferB` is `TRUE`, else `a` wins. |
| `collections::partition` | `FUNC partition OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Partition OF T` | One pass; returns a `Partition OF T` with `matched`/`unmatched` in original order. |

Predicates and other function arguments are passed as function values (for
example a named `FUNC`). The inlined filter predicates such as `isEven` cannot be
passed as values; wrap them in a `FUNC` when a predicate argument is needed.

`collections::toMap`, `collections::zipWith`, and `collections::filterEntries`
(plan-01-functions.md §6.4) are not yet provided: they require storing the
compiler-owned `MapEntry` record inside a `List` and applying a two-argument
function value element-wise, which the current runtime does not support. They are
deferred until that infrastructure lands.

| `collections::toMap` | `FUNC toMap OF K, V(value AS List OF MapEntry OF K TO V) AS Map OF K TO V` | Builds a map from entries. `K` must be comparable. On duplicate keys, **last entry wins**. |
| `collections::zipWith` | `FUNC zipWith OF A, B, U(a AS List OF A, b AS List OF B, f AS FUNC(A, B) AS U) AS List OF U` | Combines items position-wise through `f`. Stops at the shorter input. |
| `collections::filterEntries` | `FUNC filterEntries OF K, V(value AS Map OF K TO V, predicate AS FUNC(MapEntry OF K TO V) AS Boolean) AS Map OF K TO V` | Keeps entries where `predicate` returns `TRUE`. |


## 4. Built-in Filter Functions

These predicate helpers are always in scope and are intended for use with `filter`, `MATCH` guards, and ordinary conditionals.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `isEven` | `FUNC isEven(value AS Integer) AS Boolean` | `TRUE` when `value MOD 2 = 0`. |
| `isOdd` | `FUNC isOdd(value AS Integer) AS Boolean` | `TRUE` when `value MOD 2 <> 0`. |
| `isPositive` | `FUNC isPositive(value AS Integer) AS Boolean` | `TRUE` when `value > 0`. |
| `isPositive` | `FUNC isPositive(value AS Float) AS Boolean` | `TRUE` when `value > 0.0`. |
| `isPositive` | `FUNC isPositive(value AS Fixed) AS Boolean` | `TRUE` when `value > 0.0`. |
| `isNegative` | `FUNC isNegative(value AS Integer) AS Boolean` | `TRUE` when `value < 0`. |
| `isNegative` | `FUNC isNegative(value AS Float) AS Boolean` | `TRUE` when `value < 0.0`. |
| `isNegative` | `FUNC isNegative(value AS Fixed) AS Boolean` | `TRUE` when `value < 0.0`. |
| `isZero` | `FUNC isZero(value AS Integer) AS Boolean` | `TRUE` when `value = 0`. |
| `isZero` | `FUNC isZero(value AS Float) AS Boolean` | `TRUE` when `value = 0.0`. |
| `isZero` | `FUNC isZero(value AS Fixed) AS Boolean` | `TRUE` when `value = 0.0`. |
| `isEmpty` | `FUNC isEmpty(value AS String) AS Boolean` | `TRUE` when `len(value) = 0`. |
| `isEmpty` | `FUNC isEmpty OF T(value AS List OF T) AS Boolean` | `TRUE` when `len(value) = 0`. |
| `isEmpty` | `FUNC isEmpty OF K, V(value AS Map OF K TO V) AS Boolean` | `TRUE` when `len(value) = 0`. |
| `isNotEmpty` | `FUNC isNotEmpty(value AS String) AS Boolean` | `TRUE` when `len(value) > 0`. |
| `isNotEmpty` | `FUNC isNotEmpty OF T(value AS List OF T) AS Boolean` | `TRUE` when `len(value) > 0`. |
| `isNotEmpty` | `FUNC isNotEmpty OF K, V(value AS Map OF K TO V) AS Boolean` | `TRUE` when `len(value) > 0`. |

## 5. Strings Package

String helpers are exported by the `strings` package. Package functions are called with their package qualifier.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `strings::trim` | `FUNC trim(value AS String) AS String` | Removes leading and trailing Unicode whitespace. |
| `strings::trimStart` | `FUNC trimStart(value AS String) AS String` | Removes leading Unicode whitespace. |
| `strings::trimEnd` | `FUNC trimEnd(value AS String) AS String` | Removes trailing Unicode whitespace. |
| `strings::upper` | `FUNC upper(value AS String) AS String` | Converts to uppercase using Unicode case mapping. |
| `strings::lower` | `FUNC lower(value AS String) AS String` | Converts to lowercase using Unicode case mapping. |
| `strings::caseFold` | `FUNC caseFold(value AS String) AS String` | Applies Unicode case folding for caseless comparison. |
| `strings::normalizeNfc` | `FUNC normalizeNfc(value AS String) AS String` | Returns Unicode NFC-normalized text. |
| `strings::graphemes` | `FUNC graphemes(value AS String) AS List OF String` | Splits `value` into extended grapheme clusters. |
| `strings::startsWith` | `FUNC startsWith(value AS String, prefix AS String) AS Boolean` | `TRUE` when `value` begins with `prefix`. |
| `strings::endsWith` | `FUNC endsWith(value AS String, suffix AS String) AS Boolean` | `TRUE` when `value` ends with `suffix`. |
| `strings::contains` | `FUNC contains(value AS String, needle AS String) AS Boolean` | `TRUE` when `needle` appears in `value`. |
| `strings::split` | `FUNC split(value AS String, delimiter AS String) AS List OF String` | Splits `value` by `delimiter`. |
| `strings::join` | `FUNC join(parts AS List OF String, delimiter AS String) AS String` | Joins strings with `delimiter`. |
| `strings::byteLen` | `FUNC byteLen(value AS String) AS Integer` | Number of bytes required to encode `value` as UTF-8. |
| `strings::find` | `FUNC find(value AS String, needle AS String, start AS Integer = 0) AS Integer` | Zero-based scalar index of the first occurrence at or after `start`. Fails with `errorCode::ErrNotFound` (`77050004`) when absent and `77050001` when `start` is out of range. |
| `strings::mid` | `FUNC mid(value AS String, start AS Integer, count AS Integer) AS String` | Returns a substring by zero-based Unicode scalar index. Fails with `77050001` on invalid range. |
| `strings::replace` | `FUNC replace(value AS String, old AS String, new AS String) AS String` | Replaces all non-overlapping occurrences. |
| `strings::startsWithAny` | `FUNC startsWithAny(value AS String, prefixes AS List OF String) AS Boolean` | `TRUE` when `value` begins with any string in `prefixes`. Empty list → `FALSE`. Total. |
| `strings::endsWithAny` | `FUNC endsWithAny(value AS String, suffixes AS List OF String) AS Boolean` | `TRUE` when `value` ends with any string in `suffixes`. Empty list → `FALSE`. Total. |
| `strings::stripPrefix` | `FUNC stripPrefix(value AS String, prefix AS String) AS String` | Returns `value` with one leading `prefix` removed if present; otherwise `value` unchanged. Total. |
| `strings::stripSuffix` | `FUNC stripSuffix(value AS String, suffix AS String) AS String` | Returns `value` with one trailing `suffix` removed if present; otherwise unchanged. Total. |
| `strings::count` | `FUNC count(value AS String, needle AS String) AS Integer` | Number of non-overlapping occurrences of `needle`. Fails with `ErrInvalidArgument` (`77050002`) when `needle` is empty. |
| `strings::left` | `FUNC left(value AS String, count AS Integer) AS String` | First `count` scalars. Clamps when `count >= len(value)`; `count = 0` → `""`. Fails with `77050002` when `count < 0`. |
| `strings::right` | `FUNC right(value AS String, count AS Integer) AS String` | Last `count` scalars. Clamps when `count >= len(value)`; `count = 0` → `""`. Fails with `77050002` when `count < 0`. |
| `strings::repeat` | `FUNC repeat(value AS String, times AS Integer) AS String` | `value` concatenated `times` times. `times = 0` → `""`. Fails with `77050002` when `times < 0`. |
| `strings::padLeft` | `FUNC padLeft(value AS String, width AS Integer, padChar AS String = " ") AS String` | Left-pads with `padChar` to a total scalar width of `width`. No change when `len(value) >= width`. `padChar` must be exactly one Unicode scalar, else `77050002`; `width < 0` fails `77050002`. |
| `strings::padRight` | `FUNC padRight(value AS String, width AS Integer, padChar AS String = " ") AS String` | Right-pads, same rules as `padLeft`. |
| `strings::graphemeAt` | `FUNC graphemeAt(value AS String, index AS Integer) AS String` | The extended grapheme cluster at zero-based grapheme `index`. Fails with `ErrIndexOutOfRange` (`77050001`) when out of range. |
| `strings::graphemesCount` | `FUNC graphemesCount(value AS String) AS Integer` | Number of extended grapheme clusters. |
| `strings::trimChars` | `FUNC trimChars(value AS String, chars AS String) AS String` | Removes any leading/trailing scalars that appear in the set `chars`. `chars = ""` → `value` unchanged. |

## 6. Regex Package

Regular-expression helpers are exported by the `regex` package. Package functions are called with their package qualifier.

The regular-expression dialect is **MFBASIC's own**, defined completely and
self-containedly in `specifications/regex.md` — its grammar, matching semantics,
escapes, class shorthands, flags, replacement mini-language, and error behavior
are normative there, not defined by reference to Rust, PCRE, POSIX, or any host
`regcomp()`. It is compiler-defined and produces byte-for-byte identical results
across every target and on both the native and Binary Representation code paths
(`regex.md` §16). Invalid patterns fail with `ErrInvalidFormat`.

Matching is Unicode-aware and user-visible indexes remain zero-based Unicode
scalar indexes, not byte offsets (`regex.md` §2). Key properties:

- syntax and matching behavior are defined by `specifications/regex.md`
- backreferences and look-around are not supported (`regex.md` §15)
- behavior must not vary by target libc or OS regex library
- replacement behavior follows the replacement mini-language in `regex.md` §13

| Function | Signature | Behavior |
|----------|-----------|----------|
| `regex::match` | `FUNC match(value AS String, pattern AS String) AS Boolean` | `TRUE` when `pattern` matches anywhere in `value`. |
| `regex::find` | `FUNC find(value AS String, pattern AS String, start AS Integer = 0) AS Integer` | Returns the zero-based scalar index of the first regex match at or after `start`, or `-1` when there is no match. Fails with `ErrIndexOutOfRange` when `start` is outside `0 .. len(value)`. |
| `regex::findAll` | `FUNC findAll(value AS String, pattern AS String, start AS Integer = 0) AS List OF Integer` | Returns the zero-based scalar start index of every non-overlapping match at or after `start`, left to right. Returns an empty list when there are none (it does not fail with `ErrNotFound`). |
| `regex::replace` | `FUNC replace(value AS String, pattern AS String, replacement AS String) AS String` | Replaces all regex matches. |

## 7. Built-in IO Package

Terminal and standard-stream I/O is provided by the `io` package. Package functions are called with their package qualifier. Structured terminal / TUI control (cursor, color, attributes, screen clearing, and the terminal size) is provided by the separate `term` package; see `specifications/plan-01-term.md`.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `io::print` | `FUNC print(value AS String) AS Nothing` | Writes `value` to standard output and appends a newline. Fails with `77020002` on output failure. |
| `io::write` | `FUNC write(value AS String) AS Nothing` | Writes `value` to standard output without appending a newline. Fails with `77020002` on output failure. |
| `io::printError` | `FUNC printError(value AS String) AS Nothing` | Writes `value` to standard error and appends a newline. Fails with `77020002` on output failure. |
| `io::writeError` | `FUNC writeError(value AS String) AS Nothing` | Writes `value` to standard error without appending a newline. Fails with `77020002` on output failure. |
| `io::flush` | `FUNC flush() AS Nothing` | Flushes standard output. Fails with `77020002` on output failure. |
| `io::flushError` | `FUNC flushError() AS Nothing` | Flushes standard error. Fails with `77020002` on output failure. |
| `io::input` | `FUNC input(prompt AS String = "") AS String` | Writes `prompt` to standard output when non-empty, flushes standard output, reads and echoes one terminal line until newline, and returns it without the line terminator. Fails with `77020003` at EOF, `77020004` on invalid UTF-8 input, and `77020005` on input failure. |
| `io::readLine` | `FUNC readLine() AS String` | Reads one line from standard input without terminal echo and returns it without the line terminator. It waits for newline on interactive terminals. Fails with `77020003` at EOF, `77020004` on invalid UTF-8 input, and `77020005` on input failure. |
| `io::readChar` | `FUNC readChar() AS String` | Reads one Unicode scalar value from standard input without waiting for newline and without terminal echo. Fails with `77020003` at EOF, `77020004` on invalid UTF-8, and `77020005` on input failure. |
| `io::readByte` | `FUNC readByte() AS Byte` | Reads one byte from standard input without waiting for newline and without terminal echo. Fails with `77020003` at EOF and `77020005` on input failure. |
| `io::pollInput` | `FUNC pollInput(timeoutMs AS Integer = 0) AS Boolean` | Waits until standard input can be read without blocking. `timeoutMs < 0` waits forever, `timeoutMs = 0` performs a nonblocking readiness check, and `timeoutMs > 0` waits up to that many milliseconds. Returns `TRUE` when input is ready and `FALSE` on timeout. Fails with `77020005` on input polling failure. |
| `io::isInputTerminal` | `FUNC isInputTerminal() AS Boolean` | `TRUE` when standard input is attached to an interactive terminal. |
| `io::isOutputTerminal` | `FUNC isOutputTerminal() AS Boolean` | `TRUE` when standard output is attached to an interactive terminal. |
| `io::isErrorTerminal` | `FUNC isErrorTerminal() AS Boolean` | `TRUE` when standard error is attached to an interactive terminal. |

The terminal size is reported by `term::terminalSize() AS TermSize` (requires TUI mode; see `specifications/plan-01-term.md`), replacing the former `io::terminalSize`.

On interactive terminals, `io::readLine`, `io::readChar`, and `io::readByte` temporarily disable terminal echo while reading, then restore the previous terminal mode before returning or failing. `io::readChar` and `io::readByte` also temporarily disable canonical input so each keypress is delivered immediately. When standard input is not an interactive terminal, these functions read from the stream directly.

There is no `PRINT` statement and no trailing-semicolon newline suppression. Use `io::print` for newline-terminated standard output, `io::write` for standard output without a newline, `io::printError` or `io::writeError` for standard error, and `fs::writeAll` for file-handle output.

Use `toString` explicitly before calling `io::print`, `io::write`, `io::printError`, or `io::writeError` when outputting a non-string value. Output functions are intended for user-visible text and diagnostics, not automatic structured logging of arbitrary values.

## 8. Built-in Filesystem Package

Filesystem and file-handle functions live in the `fs` package. Paths are `String` values.

One-shot path operations read, write, inspect, or modify filesystem entries without exposing resource handles. File-handle I/O uses `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, or `fs::createTempFile`; the resulting `File` is closed automatically by lexical drop when its binding leaves scope, on every exit path, or by an explicit `fs::close`.

```basic
RES file = fs::open("data.txt", "read")
' file is in scope here
' file is closed by lexical drop when this scope ends
```

Symlink behavior is explicit:

- `fs::readText`, `fs::writeText`, `fs::appendText`, `fs::fileExists`, `fs::directoryExists`, `fs::exists`, and `fs::listDirectory` follow symlinks in the final path component and during directory traversal.
- `fs::deleteFile` deletes the symlink itself when the final path component is a symlink; it does not delete the symlink target.
- `fs::deleteDirectory` deletes the directory entry named by the final path only when it is an actual empty directory; a symlink to a directory is not treated as a directory for deletion.
- Create operations fail with `ErrAlreadyExists` when the final path already exists, including when it is a symlink.
- Use `fs::openFileNoFollow` when the final component must not be a symlink, and use `fs::canonicalPath` plus `fs::isWithin` to validate path containment before accessing user-controlled paths.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `fs::fileExists` | `FUNC fileExists(path AS String) AS Boolean` | `TRUE` when `path` exists and is a regular file. |
| `fs::directoryExists` | `FUNC directoryExists(path AS String) AS Boolean` | `TRUE` when `path` exists and is a directory. |
| `fs::exists` | `FUNC exists(path AS String) AS Boolean` | `TRUE` when any filesystem entry exists at `path`. |
| `fs::readBytes` | `FUNC readBytes(path AS String) AS List OF Byte` | Reads a file as raw bytes. Fails with `77030001`, `77030002`, or `77020001`. |
| `fs::readText` | `FUNC readText(path AS String) AS String` | Reads a UTF-8 text file. Fails with `77030001`, `77030002`, `77020001`, or `77020004`. |
| `fs::writeBytes` | `FUNC writeBytes(path AS String, bytes AS List OF Byte) AS Nothing` | Writes raw bytes to a file, replacing any existing file. Fails with `77030002`, `77030003`, or `77020002`. |
| `fs::writeText` | `FUNC writeText(path AS String, value AS String) AS Nothing` | Writes a UTF-8 text file, replacing any existing file. Fails with `77030002`, `77030003`, or `77020002`. |
| `fs::writeBytesAtomic` | `FUNC writeBytesAtomic(path AS String, bytes AS List OF Byte) AS Nothing` | Writes raw bytes to a temporary file in the same directory, flushes it, then atomically replaces `path` when the host filesystem supports atomic rename. Fails rather than falling back to a non-atomic replace. |
| `fs::writeTextAtomic` | `FUNC writeTextAtomic(path AS String, value AS String) AS Nothing` | Writes UTF-8 text to a temporary file in the same directory, flushes it, then atomically replaces `path` when the host filesystem supports atomic rename. Fails rather than falling back to a non-atomic replace. |
| `fs::appendBytes` | `FUNC appendBytes(path AS String, bytes AS List OF Byte) AS Nothing` | Appends raw bytes to a file, creating it when needed. Fails with `77030002`, `77030003`, or `77020002`. |
| `fs::appendText` | `FUNC appendText(path AS String, value AS String) AS Nothing` | Appends UTF-8 text to a file, creating it when needed. Fails with `77030002`, `77030003`, or `77020002`. |
| `fs::open` | `FUNC open(path AS String, mode AS String) AS File` | Opens a file handle, closed by lexical drop or `fs::close`. Portable modes are `"read"`/`"r"`, `"write"`/`"w"`, `"readWrite"`/`"rw"`, and `"append"`/`"a"`. Invalid modes, empty paths, and embedded NUL bytes fail with `ErrInvalidArgument` (`77050002`). Missing files fail with `ErrNotFound` (`77050004`) for read-style opens. |
| `fs::openFile` | `FUNC openFile(path AS String, mode AS String = "read") AS File` | Opens a file handle. Portable modes are `"read"`/`"r"`, `"write"`/`"w"`, `"readWrite"`/`"rw"`, and `"append"`/`"a"`. Fails with `77030001`, `77030002`, or `77030003`. |
| `fs::openFileNoFollow` | `FUNC openFileNoFollow(path AS String, mode AS String = "read") AS File` | Opens a file handle like `fs::openFile`, with the same portable modes, but fails with `ErrAccessDenied` when the final path component is a symlink. |
| `fs::createTempFile` | `FUNC createTempFile() AS File`<br>`FUNC createTempFile(directory AS String) AS File` | Securely creates and opens a new unique file named `mfb-<uuid>.tmp`. Without `directory`, the file is created in the OS temp directory. With `directory`, the file is created in that directory. The caller owns the returned `File`. |
| `fs::tempDirectory` | `FUNC tempDirectory() AS String` | Returns the OS temp directory. macOS uses `_confstr(_CS_DARWIN_USER_TEMP_DIR)`. Linux uses `$TMPDIR` when set and non-empty, otherwise `/tmp`. |
| `fs::readLine` | `FUNC readLine(file AS File) AS String` | Reads one line without the line terminator. Fails with `77020003` at EOF and `77020001` on read failure. |
| `fs::readAll` | `FUNC readAll(file AS File) AS String` | Reads the rest of the file as UTF-8 text. Fails with `77020001` on read failure and `77020004` on invalid UTF-8. |
| `fs::readAllBytes` | `FUNC readAllBytes(file AS File) AS List OF Byte` | Reads the rest of the file as raw bytes. Fails with `77020001` on read failure. |
| `fs::writeAll` | `FUNC writeAll(file AS File, value AS String) AS Nothing` | Writes all text to `file`. Fails with `77020002` on write failure. |
| `fs::writeAllBytes` | `FUNC writeAllBytes(file AS File, bytes AS List OF Byte) AS Nothing` | Writes all bytes to `file`. Fails with `77020002` on write failure. |
| `fs::close` | `FUNC close(file AS File) AS Nothing` | Closes a file handle. Calling it more than once is an error. |
| `fs::eof` | `FUNC eof(file AS File) AS Boolean` | `TRUE` when the next read would be at end of file. |
| `fs::canonicalPath` | `FUNC canonicalPath(path AS String) AS String` | Returns an absolute normalized path after resolving `.`/`..` and symlinks for every existing component. Fails when the path or a required parent does not exist. |
| `fs::isWithin` | `FUNC isWithin(base AS String, child AS String) AS Boolean` | Canonicalizes both paths and returns `TRUE` only when `child` is equal to `base` or is contained below `base`. |
| `fs::pathJoin` | `FUNC pathJoin(parts AS List OF String) AS String` | Joins path components using the host platform's separator and normal separator rules. |
| `fs::pathDirName` | `FUNC pathDirName(path AS String) AS String` | Returns the directory portion of `path` without accessing the filesystem. |
| `fs::pathBaseName` | `FUNC pathBaseName(path AS String) AS String` | Returns the final path component without accessing the filesystem. |
| `fs::pathExtension` | `FUNC pathExtension(path AS String) AS String` | Returns the final component's extension, including the leading dot when present, without accessing the filesystem. |
| `fs::pathNormalize` | `FUNC pathNormalize(path AS String) AS String` | Normalizes separators and `.`/`..` components syntactically without resolving symlinks or requiring the path to exist. |
| `fs::deleteFile` | `FUNC deleteFile(path AS String) AS Nothing` | Deletes a regular file. Fails with `77030001` when missing. |
| `fs::createDirectory` | `FUNC createDirectory(path AS String) AS Nothing` | Creates one directory. Fails if the parent is missing or the target already exists. |
| `fs::createDirectories` | `FUNC createDirectories(path AS String) AS Nothing` | Creates a directory and any missing parents. |
| `fs::deleteDirectory` | `FUNC deleteDirectory(path AS String) AS Nothing` | Deletes an empty directory. Fails with `77030001` when missing and `77030005` when the directory is not empty. |
| `fs::listDirectory` | `FUNC listDirectory(path AS String) AS List OF String` | Lists direct child names in implementation-defined stable order; the native runtime returns them sorted ascending by UTF-8 byte value. |
| `fs::currentDirectory` | `FUNC currentDirectory() AS String` | Returns the current working directory. |
| `fs::setCurrentDirectory` | `FUNC setCurrentDirectory(path AS String) AS Nothing` | Changes the current working directory. |

`File` is an opaque standard `RESOURCE` type and unique handle. It is closed automatically with `fs::close` by lexical drop when its binding leaves scope, on every exit path, and may also be closed explicitly with `fs::close`. `File` is thread-transferable: it crosses a thread boundary through `thread::transfer` (the resource plane), which moves ownership to the destination side so the sender cannot use the handle again on a successful path. Resources are not valid `thread::send` messages.

`File`, `Socket`, and `Listener` are the standard built-in resource types. Beyond these, a **binding package may introduce its own native resource types** through a package-scope `RESOURCE <Name> CLOSE BY <pkg>::close` declaration paired with a `LINK` block (mfbasic.md §17). An imported native resource behaves exactly like a standard one: it is bound with `RES`, borrowed at ordinary calls, auto-closed by lexical drop through its registered close op, never copied/stored/field-accessed, and reported by `mfb audit` — and is thread-sendable only when the binding declares `THREAD_SENDABLE`. Diagnostics specific to declaring native bindings are listed in `specifications/error_codes.md` (`1-102-0008`–`0009`, `2-203-0089`–`0098`).

## 9. Built-in Thread Package

Thread functions live in the `thread` package. Thread entry points must be exported `ISOLATED FUNC` declarations from imported packages with type `ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out`; lambdas, closures, `SUB`s, non-isolated functions, current-package functions, and functions without the leading worker handle parameter are rejected at compile time.

Thread boundary types must be thread-sendable. `thread::start` requires `In`, `Msg`, and `Out` to be thread-sendable. The message channel is **resource-free**: both `thread::send` overloads require `Msg` to be thread-sendable **and not a resource** (resources cross via `thread::transfer` / `thread::accept` — the resource plane), and worker return values require `Out` to be thread-sendable. Thread sendability is a type metadata property, not a per-value flag. Primitive owned values, strings, and standard immutable containers are sendable when their contained types are sendable. Records and unions are sendable only when all field or payload types are sendable, and a worker outcome (internally a fallible result) is sendable when its success type is. Opaque handles are not sendable by default; each concrete handle type opts in explicitly. `Thread`, `ThreadWorker`, and `Listener` are not thread-sendable.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `thread::start` | `FUNC start OF In, Msg, Out(f AS ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out, data AS In, inboundLimit AS Integer = 64, outboundLimit AS Integer = 64) AS Thread OF Msg TO Out` | Starts imported package export `f` in a fresh package instance with bounded inbound and outbound queues, passing the worker-side thread handle and `data` into the thread by copy, move, or freeze. `In`, `Msg`, and `Out` must be thread-sendable. Current-package functions are rejected at compile time. Fails with `77050002` for queue limits below `1`. |
| `thread::isRunning` | `FUNC isRunning OF Msg, Out(t AS Thread OF Msg TO Out) AS Boolean` | `TRUE` while the thread entry function is still running. |
| `thread::waitFor` | `FUNC waitFor OF Msg, Out(t AS Thread OF Msg TO Out) AS Out` | Waits for completion, retrieves the thread's stored result, closes the parent `Thread` handle, then returns the successful `Out`. `Err` auto-propagates like any fallible function. Later use of the same handle fails with `ErrResourceClosed`. |
| `thread::cancel` | `FUNC cancel OF Msg, Out(t AS Thread OF Msg TO Out) AS Nothing` | Requests cooperative cancellation. New parent-side sends fail after cancellation is requested, and runtime-managed worker queue waits inside the worker wake with `ErrInterrupted`. |
| `thread::send` | `FUNC send OF Msg, Out(t AS Thread OF Msg TO Out, data AS Msg, timeoutMs AS Integer = 0) AS Nothing` | Sends thread-sendable `data` to the thread's inbound queue by copy, move, or freeze. `timeoutMs = 0` does not wait for queue space. Fails if the thread has ended, cancellation was requested, the timeout expires, or the timeout is invalid. |
| `thread::send` | `FUNC send OF Msg, Out(t AS ThreadWorker OF Msg TO Out, data AS Msg, timeoutMs AS Integer = 0) AS Nothing` | Sends thread-sendable `data` from the worker to the parent-visible outbound queue by copy, move, or freeze. `timeoutMs = 0` does not wait for queue space. Fails if the thread has ended, the timeout expires, or the timeout is invalid. |
| `thread::poll` | `FUNC poll OF Msg, Out(t AS Thread OF Msg TO Out, ms AS Integer) AS Boolean` | Waits up to `ms` milliseconds for an outbound message. Returns `TRUE` when `thread::receive(t)` can read without blocking. |
| `thread::receive` | `FUNC receive OF Msg, Out(t AS Thread OF Msg TO Out, timeoutMs AS Integer = 0) AS Msg` | Parent-side read of the next outbound message by copy, move, or freeze. `timeoutMs = 0` does not wait for a message. Fails when no message is available before the timeout. |
| `thread::receive` | `FUNC receive OF Msg, Out(t AS ThreadWorker OF Msg TO Out, timeoutMs AS Integer = 0) AS Msg` | Worker-side read from the inbound queue for `t`. Valid only inside the worker thread. `timeoutMs = 0` does not wait; `timeoutMs = -1` waits indefinitely until a message, queue closure, or cancellation. Other negative timeouts fail with `ErrInvalidArgument`. Cancellation fails with `ErrInterrupted`. |
| `thread::isCancelled` | `FUNC isCancelled OF Msg, Out(t AS ThreadWorker OF Msg TO Out) AS Boolean` | Worker-side cancellation check. Returns `TRUE` after the parent requests cancellation. |
| `thread::transfer` | `FUNC transfer OF Msg, Res, Out(t AS Thread OF Msg RES Res TO Out, res AS RES Res, timeoutMs AS Integer = 0) AS Nothing` | Resource plane (a dedicated per-thread queue): **move** a thread-sendable resource `res` to the thread. Consumes the sender binding; ownership returns to the sender on failure. The thread must declare a resource plane `RES Res` (`TYPE_THREAD_NOT_SENDABLE`/argument mismatch otherwise). |
| `thread::accept` | `FUNC accept OF Msg, Res, Out(t AS Thread OF Msg RES Res TO Out, timeoutMs AS Integer = 0) AS RES Res` | Resource plane: receive a transferred resource and bind it with `RES`; its `STATE` is declared on the binding and moves with the resource. |
| `thread::transfer` | `FUNC transfer OF Msg, Res, Out(t AS ThreadWorker OF Msg RES Res TO Out, res AS RES Res, timeoutMs AS Integer = 0) AS Nothing` | Worker-side resource transfer to the parent-visible plane. |
| `thread::accept` | `FUNC accept OF Msg, Res, Out(t AS ThreadWorker OF Msg RES Res TO Out, timeoutMs AS Integer = 0) AS RES Res` | Worker-side receive of a transferred resource. |

When a thread ends, its `Thread` value owns the completed worker outcome until it is retrieved. `thread::waitFor(t)` waits for that outcome to exist, auto-unwraps the `Out` value or auto-propagates the `Error` like any other call, and closes the parent `Thread` handle. Retrieval is one-shot; any later use of the same `Thread` handle fails with `ErrResourceClosed`.

`Thread` is a non-copyable owned handle. Lexical cleanup drops live parent `Thread` handles on normal scope exit, `RETURN`, `EXIT`/`CONTINUE`, `FAIL`, `PROPAGATE`, auto-propagated errors, and trap routing; `EXIT PROGRAM` unwinds and drops live thread handles in every caller frame before process termination. Dropping a completed `Thread` releases the unretrieved result and any remaining queued parent-visible messages. Dropping a running `Thread` requests cancellation, wakes and closes the queues, detaches the worker, and leaves final runtime reclamation to worker completion. Reassigning a `MUT Thread` evaluates the replacement first, then drops the old handle before storing the new one. A binding moved out by return or another consuming operation is not dropped by the source scope. Compiler-generated cleanup is idempotent for handles already closed by `thread::waitFor(t)`.

Thread functions are ordinary built-in templates. Their `Msg` and `Out` parameters are resolved by the template rules in the language specification from argument types and expected result types. `thread::start` gets `Msg` and `Out` from the started function's first `ThreadWorker OF Msg TO Out` parameter, and gets `In` from the started function's second parameter and the `data` argument. If a thread does not exchange messages, `Msg` may be `Nothing`.

`Thread` is the parent-side handle and `ThreadWorker` is the worker-side handle. `thread::send` and `thread::receive` are overloaded on those handle types to select the inbound or outbound queue. Queue timeout uses `ErrTimeout`; unavailable messages use `ErrNotFound`; cancellation uses `ErrInterrupted`.

`thread::cancel` wakes runtime-managed worker queue cancellation points inside the worker. These include `thread::receive(ThreadWorker, ...)` and `thread::send(ThreadWorker, ...)`. If cancellation is already set before entering one of these operations, it fails immediately with `ErrInterrupted`; if cancellation is requested while blocked, the runtime wakes the operation and it fails with `ErrInterrupted`. Other blocking built-ins that are implemented as runtime-managed waits, such as terminal input, blocking file reads, or network waits, must use the same cooperative error-return model when cancellation integration is provided. Cancellation does not asynchronously terminate the worker or interrupt arbitrary user/native code.

For copyable values, `thread::send` copies or freezes the sent value and the sender's original binding remains usable. For non-copyable thread-sendable values, including sendable resource handles, a successful `thread::send` moves ownership to the destination side immediately. While the value is queued, the destination queue owns it in receiver-valid storage or runtime transfer storage independent of the sender arena. If the value is never received, the destination queue/runtime drops or closes it exactly once. If `thread::send` fails, ownership is not transferred and the sender still owns the value. A `TRAP` on the failure can release or reuse the still-owned value, distinct from the success path where the sent binding is moved. Using a moved handle is an after-move error.

Each started worker has a distinct package instance and worker arena. Thread start input, queued messages, and completed results are transferred across the thread boundary by copy, move, or freeze into storage whose lifetime is valid for the receiver. A completed result is materialized as receiver-owned storage before it is exposed through `thread::waitFor(t)`. The worker arena may be released only after the result has been transferred or the runtime keeps that arena live through result retrieval, and worker-to-parent messages have been transferred into queue storage or dropped.

## 10. Built-in Math Package

Math functions live in the `math` package. Constants are `LET` values and must be referenced as identifiers, not called like zero-argument functions. Numeric functions are overloaded by argument type; mixed numeric calls require an explicit conversion.

Math functions follow the numeric edge-case rules in §4.1. Integer and `Fixed` overflow fails with `ErrOverflow` (`77050010`). Integer and `Fixed` invalid domains, such as square root of a negative value or logarithm of a non-positive value, fail with `ErrInvalidArgument` (`77050002`). `Float` functions return only finite values: explicit domain failures fail with `ErrFloatDomain` (`77050012`), a result that would be NaN fails with `ErrFloatNaN` (`77050013`), and a result that would be infinity fails with `ErrFloatInf` (`77050014`).

| Constant                 | Type    | Value |
|--------------------------|---------|-------|
| `math::pi`               | `Float` | The mathematical constant pi as a `Float`. |
| `math::piFixed`          | `Fixed` | The mathematical constant pi rounded to the nearest `Fixed` value. |
| `math::twoOverPi`        | `Float` | The mathematical constant 2 / pi as a `Float`. |
| `math::twoOverPiFixed`   | `Fixed` | The mathematical constant 2 / pi rounded to the nearest `Fixed` value. |
| `math::pi2`              | `Float` | The mathematical constant pi / 2 as a `Float`. |
| `math::pi2Fixed`         | `Fixed` | The mathematical constant pi / 2 rounded to the nearest `Fixed` value. |
| `math::pi4`              | `Float` | The mathematical constant pi / 4 as a `Float`. |
| `math::pi4Fixed`         | `Fixed` | The mathematical constant pi / 4 rounded to the nearest `Fixed` value. |
| `math::e`                | `Float` | The mathematical constant e as a `Float`. |
| `math::eFixed`           | `Fixed` | The mathematical constant e rounded to the nearest `Fixed` value. |
| `math::ln2`              | `Float` | The mathematical constant ln(2) as a `Float`. |
| `math::ln2Fixed`         | `Fixed` | The mathematical constant ln(2) rounded to the nearest `Fixed` value. |
| `math::ln10`             | `Float` | The mathematical constant ln(10) as a `Float`. |
| `math::ln10Fixed`        | `Fixed` | The mathematical constant ln(10) rounded to the nearest `Fixed` value. |

| Function | Signature | Behavior |
|----------|-----------|----------|
| `math::abs` | `FUNC abs(value AS Integer) AS Integer` | Absolute value. Fails with `77050010` for the minimum integer overflow case. |
| `math::abs` | `FUNC abs(value AS Float) AS Float` | Absolute value. |
| `math::abs` | `FUNC abs(value AS Fixed) AS Fixed` | Absolute value. Fails with `77050010` for the minimum fixed-point overflow case. |
| `math::min` | `FUNC min(a AS Integer, b AS Integer) AS Integer` | Smaller integer. |
| `math::min` | `FUNC min(a AS Float, b AS Float) AS Float` | Smaller float. |
| `math::min` | `FUNC min(a AS Fixed, b AS Fixed) AS Fixed` | Smaller fixed-point value. |
| `math::max` | `FUNC max(a AS Integer, b AS Integer) AS Integer` | Larger integer. |
| `math::max` | `FUNC max(a AS Float, b AS Float) AS Float` | Larger float. |
| `math::max` | `FUNC max(a AS Fixed, b AS Fixed) AS Fixed` | Larger fixed-point value. |
| `math::clamp` | `FUNC clamp(value AS Integer, low AS Integer, high AS Integer) AS Integer` | Restricts `value` to `[low, high]`. Fails with `77050002` when `low > high`. |
| `math::clamp` | `FUNC clamp(value AS Float, low AS Float, high AS Float) AS Float` | Restricts `value` to `[low, high]`. Fails with `77050002` when `low > high`. |
| `math::clamp` | `FUNC clamp(value AS Fixed, low AS Fixed, high AS Fixed) AS Fixed` | Restricts `value` to `[low, high]`. Fails with `77050002` when `low > high`. |
| `math::floor` | `FUNC floor(value AS Float) AS Integer` | Greatest integer less than or equal to `value`. Fails with `77050010` when outside `Integer` range. |
| `math::floor` | `FUNC floor(value AS Fixed) AS Integer` | Greatest integer less than or equal to `value`. |
| `math::ceil` | `FUNC ceil(value AS Float) AS Integer` | Smallest integer greater than or equal to `value`. Fails with `77050010` when outside `Integer` range. |
| `math::ceil` | `FUNC ceil(value AS Fixed) AS Integer` | Smallest integer greater than or equal to `value`. |
| `math::round` | `FUNC round(value AS Float) AS Integer` | Nearest integer, halves away from zero. Fails with `77050010` when outside `Integer` range. |
| `math::round` | `FUNC round(value AS Fixed) AS Integer` | Nearest integer, halves away from zero. |
| `math::sqrt` | `FUNC sqrt(value AS Float) AS Float` | Square root. Fails with `77050012` for negative input. |
| `math::sqrt` | `FUNC sqrt(value AS Fixed) AS Fixed` | Fixed-point square root rounded to nearest `Fixed`. Fails with `77050002` for negative input. |
| `math::pow` | `FUNC pow(base AS Float, exponent AS Float) AS Float` | Power function. |
| `math::pow` | `FUNC pow(base AS Fixed, exponent AS Fixed) AS Fixed` | Fixed-point power rounded to nearest `Fixed`. Fails with `77050002` for invalid domains and `77050010` on overflow. |
| `math::exp` | `FUNC exp(value AS Float) AS Float` | e raised to `value`. |
| `math::exp` | `FUNC exp(value AS Fixed) AS Fixed` | Fixed-point e raised to `value`, rounded to nearest `Fixed`. Fails with `77050010` on overflow. |
| `math::log` | `FUNC log(value AS Float) AS Float` | Natural logarithm. Fails with `77050012` for non-positive input. |
| `math::log` | `FUNC log(value AS Fixed) AS Fixed` | Fixed-point natural logarithm rounded to nearest `Fixed`. Fails with `77050002` for non-positive input. |
| `math::log10` | `FUNC log10(value AS Float) AS Float` | Base-10 logarithm. Fails with `77050012` for non-positive input. |
| `math::log10` | `FUNC log10(value AS Fixed) AS Fixed` | Fixed-point base-10 logarithm rounded to nearest `Fixed`. Fails with `77050002` for non-positive input. |
| `math::sin` | `FUNC sin(value AS Float) AS Float` | Sine, radians. |
| `math::sin` | `FUNC sin(value AS Fixed) AS Fixed` | Fixed-point sine, radians, rounded to nearest `Fixed`. |
| `math::cos` | `FUNC cos(value AS Float) AS Float` | Cosine, radians. |
| `math::cos` | `FUNC cos(value AS Fixed) AS Fixed` | Fixed-point cosine, radians, rounded to nearest `Fixed`. |
| `math::tan` | `FUNC tan(value AS Float) AS Float` | Tangent, radians. |
| `math::tan` | `FUNC tan(value AS Fixed) AS Fixed` | Fixed-point tangent, radians, rounded to nearest `Fixed`. Fails with `77050002` at undefined points. |
| `math::asin` | `FUNC asin(value AS Float) AS Float` | Arc sine. Fails with `77050012` when outside `[-1.0, 1.0]`. |
| `math::asin` | `FUNC asin(value AS Fixed) AS Fixed` | Fixed-point arc sine rounded to nearest `Fixed`. Fails with `77050002` when outside `[-1.0, 1.0]`. |
| `math::acos` | `FUNC acos(value AS Float) AS Float` | Arc cosine. Fails with `77050012` when outside `[-1.0, 1.0]`. |
| `math::acos` | `FUNC acos(value AS Fixed) AS Fixed` | Fixed-point arc cosine rounded to nearest `Fixed`. Fails with `77050002` when outside `[-1.0, 1.0]`. |
| `math::atan` | `FUNC atan(value AS Float) AS Float` | Arc tangent. |
| `math::atan` | `FUNC atan(value AS Fixed) AS Fixed` | Fixed-point arc tangent rounded to nearest `Fixed`. |
| `math::atan2` | `FUNC atan2(y AS Float, x AS Float) AS Float` | Two-argument arc tangent using the standard `atan2(y, x)` convention. |
| `math::atan2` | `FUNC atan2(y AS Fixed, x AS Fixed) AS Fixed` | Fixed-point two-argument arc tangent using the standard `atan2(y, x)` convention, rounded to nearest `Fixed`. |
| `math::rand` | `FUNC rand(min AS Integer, max AS Integer) AS Integer` | Uniformly distributed pseudo-random integer in the inclusive range `[min, max]`. Fails with `77050002` when `min > max`. |
| `math::seed` | `FUNC seed(value AS Integer) AS Nothing` | Reseeds the calling thread's random generator. A fixed seed makes the subsequent `math::rand` sequence reproducible. |

### 10.1 Random Number Generation

`math::rand` draws from a per-thread [PCG64](https://www.pcg-random.org/) (XSL-RR 128/64) generator. The generator state is owned by each thread independently, so concurrent threads never share or contend on it.

Each thread is seeded automatically:

- The program's main thread is seeded from the operating system's entropy pool at startup, so an unseeded program produces a different `math::rand` sequence on every run.
- A thread spawned with `thread::start` receives its own stream by drawing a fresh seed from the spawning thread's generator. This keeps each thread's sequence independent while remaining reproducible when the spawning thread has been explicitly seeded.

`math::seed` overrides the calling thread's seed. Seeding with a fixed value makes that thread's subsequent `math::rand` results deterministic, which is useful for tests and reproducible simulations. `math::seed` affects only the thread that calls it.

## 11. Built-in Net Package

Network functions live in the `net` package. Socket handles are opaque standard `RESOURCE` types and unique handles. `net::close` runs automatically by lexical drop when a socket binding leaves scope, on every exit path, and may also be called explicitly. `Socket` and `UdpSocket` are thread-sendable and move ownership when sent through `thread::send`. `Listener` is not thread-sendable.

The package defines DNS lookup, TCP stream sockets, UDP datagram sockets, and a required early TLS package. Unix-domain sockets and detailed DNS record inspection are outside the required core package and may be provided by extension packages.

| Type | Description |
|------|-------------|
| `Socket` | Connected TCP stream socket. |
| `Listener` | TCP listening socket that accepts incoming connections. |
| `UdpSocket` | UDP datagram socket. |
| `Address` | Network endpoint: `Address[host AS String, port AS Integer]`. |
| `Datagram` | Received UDP packet: `Datagram[from AS Address, bytes AS List OF Byte]`. |
| `DatagramText` | Received UTF-8 UDP packet: `DatagramText[from AS Address, value AS String]`. |

`host` accepts a DNS name, IPv4 literal, IPv6 literal, or an empty string for all local interfaces when listening or binding UDP. `port` must be between `0` and `65535`; port `0` asks the host OS to choose an available local port.

Timeout semantics are standardized by API category:

| Category | APIs | `timeoutMs = 0` |
|----------|------|-----------------|
| Connect/open handshake | `net::connectTcp`, `tls::connect` | Use the implementation default timeout. |
| Accept/wait for peer | `net::accept` | Wait indefinitely. |
| Poll | `net::poll` | Do not wait; return immediately. |
| Socket timeout setters | `net::setReadTimeout`, `net::setWriteTimeout` | Disable that persistent read/write timeout. |
| Thread queues | `thread::send`, `thread::receive` | Do not wait for queue space or data. |

Negative timeouts are invalid and fail with `ErrInvalidArgument`.

### 10.1 DNS

| Function | Signature | Behavior |
|----------|-----------|----------|
| `net::lookup` | `FUNC lookup(host AS String, port AS Integer = 0) AS List OF Address` | Resolves `host` to one or more network addresses. Fails with `77050002`, `77070001`, `77070002`, or `77070003`. |

`net::lookup` returns implementation-defined stable ordering, typically matching the host resolver order. It does not expose DNS record types, TTLs, canonical names, or resolver metadata.

### 10.2 TCP

| Function | Signature | Behavior |
|----------|-----------|----------|
| `net::connectTcp` | `FUNC connectTcp(host AS String, port AS Integer, timeoutMs AS Integer = 0) AS Socket` | Opens a TCP connection. `timeoutMs = 0` uses the implementation default. Fails with `77050002`, `77050008`, `77070001`, `77070002`, or `77070003`. |
| `net::connectTcp` | `FUNC connectTcp(address AS Address, timeoutMs AS Integer = 0) AS Socket` | Opens a TCP connection to a resolved address. `timeoutMs = 0` uses the implementation default. Fails with `77050002`, `77050008`, `77070001`, or `77070003`. |
| `net::listenTcp` | `FUNC listenTcp(host AS String, port AS Integer, backlog AS Integer = 128) AS Listener` | Opens a TCP listener. Fails with `77050002`, `77050005`, `77050006`, `77070001`, or `77070003`. |
| `net::accept` | `FUNC accept(listener AS Listener, timeoutMs AS Integer = 0) AS Socket` | Waits for and returns the next client connection. `timeoutMs = 0` waits indefinitely; `77050008` occurs only when `timeoutMs > 0` and no client connects before the timeout expires. Fails with `77050008`, `77030004`, or `77070003`. |
| `net::poll` | `FUNC poll(sock AS Socket, timeoutMs AS Integer = 0) AS Boolean` | `TRUE` when `sock` can be read without blocking before `timeoutMs` expires. `timeoutMs = 0` polls without waiting. Fails with `77050002` or `77030004`. |
| `net::poll` | `FUNC poll(sock AS List OF Socket, timeoutMs AS Integer = 0) AS List OF Boolean` | Returns booleans aligned with `sock`; each item is `TRUE` when the socket at the same index can be read without blocking before `timeoutMs` expires. `timeoutMs = 0` polls without waiting. Fails with `77050002` or `77030004`. Not currently provided: the ownership model forbids resource handles as collection elements (§3.2), so a `List OF Socket` value cannot be constructed and this overload is unreachable. |
| `net::read` | `FUNC read(sock AS Socket, maxBytes AS Integer) AS List OF Byte` | Reads up to `maxBytes` bytes. Returns a non-empty list unless the peer closed the connection, which fails with `77070004`. Fails with `77050002`, `77050008`, `77030004`, `77070004`, or `77070005`. |
| `net::readText` | `FUNC readText(sock AS Socket, maxBytes AS Integer) AS String` | Reads bytes and decodes UTF-8 text. Fails with `77020004` on invalid UTF-8, plus the errors from `net::read`. |
| `net::write` | `FUNC write(sock AS Socket, bytes AS List OF Byte) AS Nothing` | Writes all bytes before returning. Fails with `77050008`, `77030004`, `77070004`, or `77070006`. |
| `net::writeText` | `FUNC writeText(sock AS Socket, value AS String) AS Nothing` | Encodes `value` as UTF-8 and writes all bytes. Fails with the errors from `net::write`. |
| `net::close` | `FUNC close(resource AS Socket) AS Nothing` | Closes a connected socket. Calling it more than once is an error. |
| `net::close` | `FUNC close(resource AS Listener) AS Nothing` | Closes a listener. Calling it more than once is an error. |
| `net::localAddress` | `FUNC localAddress(sock AS Socket) AS Address` | Returns the local endpoint for a connected socket. Fails with `77030004`. |
| `net::localAddress` | `FUNC localAddress(listener AS Listener) AS Address` | Returns the bound endpoint for a listener. Fails with `77030004`. |
| `net::remoteAddress` | `FUNC remoteAddress(sock AS Socket) AS Address` | Returns the peer endpoint for a connected socket. Fails with `77030004`. |
| `net::setReadTimeout` | `FUNC setReadTimeout(sock AS Socket, timeoutMs AS Integer) AS Nothing` | Sets the read timeout. `timeoutMs = 0` disables the timeout. Fails with `77050002` or `77030004`. |
| `net::setWriteTimeout` | `FUNC setWriteTimeout(sock AS Socket, timeoutMs AS Integer) AS Nothing` | Sets the write timeout. `timeoutMs = 0` disables the timeout. Fails with `77050002` or `77030004`. |

TCP reads and writes are binary by default. Text helpers are UTF-8 conveniences and do not add message framing. Programs that exchange records should define their own delimiter, length prefix, or protocol parser.

### 10.3 UDP

| Function | Signature | Behavior |
|----------|-----------|----------|
| `net::bindUdp` | `FUNC bindUdp(host AS String, port AS Integer) AS UdpSocket` | Opens a UDP socket bound to a local endpoint. Fails with `77050002`, `77050005`, `77050006`, `77070001`, or `77070003`. |
| `net::receiveFrom` | `FUNC receiveFrom(sock AS UdpSocket, maxBytes AS Integer) AS Datagram` | Receives one datagram up to `maxBytes` bytes and returns the sender address with the bytes received. Fails with `77050002`, `77030004`, `77070005`, or `77070007`. |
| `net::receiveTextFrom` | `FUNC receiveTextFrom(sock AS UdpSocket, maxBytes AS Integer) AS DatagramText` | Receives one datagram and decodes it as UTF-8 text. Fails with `77020004`, plus the errors from `net::receiveFrom`. |
| `net::sendTo` | `FUNC sendTo(sock AS UdpSocket, address AS Address, bytes AS List OF Byte) AS Nothing` | Sends one datagram to `address`. Fails with `77030004`, `77070001`, `77070003`, `77070006`, or `77070007`. |
| `net::sendTextTo` | `FUNC sendTextTo(sock AS UdpSocket, address AS Address, value AS String) AS Nothing` | Encodes `value` as UTF-8 and sends one datagram. Fails with the errors from `net::sendTo`. |
| `net::close` | `FUNC close(resource AS UdpSocket) AS Nothing` | Closes a UDP socket. Calling it more than once is an error. |
| `net::localAddress` | `FUNC localAddress(sock AS UdpSocket) AS Address` | Returns the bound local endpoint for a UDP socket. Fails with `77030004`. |
| `net::setReadTimeout` | `FUNC setReadTimeout(sock AS UdpSocket, timeoutMs AS Integer) AS Nothing` | Sets the receive timeout. `timeoutMs = 0` disables the timeout. Fails with `77050002` or `77030004`. |
| `net::setWriteTimeout` | `FUNC setWriteTimeout(sock AS UdpSocket, timeoutMs AS Integer) AS Nothing` | Sets the send timeout. `timeoutMs = 0` disables the timeout. Fails with `77050002` or `77030004`. |

UDP preserves datagram boundaries and does not guarantee delivery, ordering, or duplicate suppression. If a received datagram is larger than `maxBytes`, `receiveFrom` fails with `77070007`; implementations must not return a silently truncated datagram.

```basic
IMPORT collections
IMPORT net

LET addresses = net::lookup("example.com", 80)
LET address = collections::get(addresses, 0)

RES client = net::connectTcp(address, timeoutMs := 5000)
net::writeText(client, "ping")
LET chunk = net::readText(client, 4096)
io::print(chunk)
' client is closed by lexical drop when this scope ends
```

### 10.4 TLS Package

TLS functions live in the `tls` package. `TlsSocket` is an opaque standard `RESOURCE` type and unique handle. It wraps a connected TCP stream with certificate validation and encrypted reads/writes.

Secure defaults are mandatory:

- Certificate validation is enabled by default and uses the host trust store unless an implementation provides an explicitly configured trust store.
- Server-name validation is enabled by default. The `serverName` argument must match the certificate subject alternative name according to platform TLS rules.
- TLS versions below TLS 1.2 are disabled. Implementations should prefer TLS 1.3 when available.
- Insecure modes, custom trust stores, and certificate pinning are outside the minimal core API and must be explicit extension APIs if provided.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `tls::connect` | `FUNC connect(host AS String, port AS Integer, timeoutMs AS Integer = 0, serverName AS String = "") AS TlsSocket` | Opens a TCP connection and performs a TLS client handshake. Empty `serverName` means use `host` for certificate validation. `timeoutMs = 0` uses the implementation default. For protocols that historically used STARTTLS (SMTP, IMAP, POP3, LDAP, FTP), connect to the implicit-TLS port (465/993/995/636/990). |
| `tls::read` | `FUNC read(sock AS TlsSocket, maxBytes AS Integer) AS List OF Byte` | Reads decrypted bytes. Fails with the same read errors as `net::read`, plus TLS validation or protocol errors. |
| `tls::readText` | `FUNC readText(sock AS TlsSocket, maxBytes AS Integer) AS String` | Reads decrypted bytes and decodes UTF-8 text. |
| `tls::write` | `FUNC write(sock AS TlsSocket, bytes AS List OF Byte) AS Nothing` | Encrypts and writes all bytes before returning. |
| `tls::writeText` | `FUNC writeText(sock AS TlsSocket, value AS String) AS Nothing` | Encodes `value` as UTF-8, encrypts it, and writes all bytes. |
| `tls::close` | `FUNC close(resource AS TlsSocket) AS Nothing` | Closes the TLS session and underlying transport. Calling it more than once is an error. Implementation note (macOS, v1): `close` tears down the connection and invalidates the handle, but the per-connection dispatch queue and semaphore are reclaimed at process exit rather than immediately — a small, bounded leak for long-lived processes that open and close many TLS sessions. |

## 12. Built-in JSON Package

JSON functions live in the `json` package.

```basic
TYPE JsonNull
  value AS Nothing
END TYPE

TYPE JsonBool
  value AS Boolean
END TYPE

TYPE JsonNum
  value AS Float
END TYPE

TYPE JsonStr
  value AS String
END TYPE

TYPE JsonArr
  items AS List OF Json
END TYPE

TYPE JsonObj
  fields AS Map OF String TO Json
END TYPE

UNION Json
  JsonNull
  JsonBool
  JsonNum
  JsonStr
  JsonArr
  JsonObj
END UNION
```

The `Json` union above is a built-in package type. JSON object member order is preserved as read when possible, but lookup by key is semantic and must not depend on order.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `json::parse` | `FUNC parse(value AS String) AS Json` | Parses UTF-8 JSON text. Fails with `ErrInvalidFormat` for malformed JSON and rejects non-finite numbers. |
| `json::stringify` | `FUNC stringify(value AS Json) AS String` | Produces compact valid JSON text. Object key order is implementation-defined but stable for an unchanged `Json` value. |
| `json::get` | `FUNC get(value AS Json, path AS List OF String) AS Json` | Reads an object path from a JSON value. Fails with `ErrNotFound` when any component is absent or not an object. |
| `json::getOr` | `FUNC getOr(value AS Json, path AS List OF String, default AS Json) AS Json` | Reads an object path or returns `default` when absent. |

## 13. Built-in Error Codes

The built-in `errorCode` package exports named `Integer` constants for every standard runtime and toolchain error in the canonical registry at [error_codes.md](./error_codes.md). Programs should use these names instead of raw integer literals in source code, examples, tests, and diagnostics:

```basic
IMPORT errorCode

IF err.code = errorCode::ErrNotFound THEN
  io::print("missing")
END IF
```

Each exported constant has the same name as the registry entry and the integer value formed by removing hyphens from the canonical code string. For example, `errorCode::ErrInvalidArgument = 77050002`, `errorCode::ErrNotFound = 77050004`, and `errorCode::ErrVerificationFailed = 33020001`.

System-defined error codes are reserved for the language, compiler, toolchain, and standard package. The hyphenated `G-SSS-EEEE` form is the canonical representation in specifications and diagnostics. The integer `Error.code` payload uses the same digits without hyphens. User programs and third-party packages should reserve their own ranges by convention and should not reuse system-defined codes.

The master registry in [error_codes.md](./error_codes.md) is the only normative source for:

- Runtime and standard package `Error` values
- Compiler diagnostic rule codes
- Toolchain and package-manager diagnostics

Project source-discovery failures such as selected-file overlaps and outside-project path resolution use those registered compiler diagnostic codes rather than runtime `Error` values.

Runtime `Error` values produced by the standard package should use the runtime registry entries where possible. Compiler and toolchain codes are diagnostics; they are not normally produced by running MFBASIC programs. For example, `TYPE_MATCH_NOT_EXHAUSTIVE` is emitted by the compiler when a `MATCH` lacks complete static coverage, not as a runtime `Error` value.

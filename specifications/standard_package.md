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
| `Error` | Standard error payload: `Error[code AS Integer, message AS String]`. |
| `Result OF T` | Standard success/error union: private `Ok OF T` or public `Error`. |
| `MapEntry OF K TO V` | Standard map iteration entry: `MapEntry[key AS K, value AS V]`. |
| `Thread OF Msg TO Out` | Opaque handle to an isolated thread with message type `Msg` and result type `Out`. |

The `Error` and `Result` shapes are built into the language:

```basic
TYPE Error
  code    AS Integer
  message AS String
END TYPE
```

`Result OF T` is compiler-owned notation. It describes the built-in `Result` template with a private success member `Ok OF T` and a public error member `Error`; concrete uses are monomorphized before bytecode generation. Users may construct `Error[...]`, but may not construct `Result` or `Ok` directly. Because `Result` is a built-in union shape, it is not eligible for uninitialized `MUT` defaults.

`MapEntry OF K TO V` is a compiler-owned record shape produced by iterating a `Map OF K TO V`. It has public read-only fields:

```basic
TYPE MapEntry OF K TO V
  key AS K
  value AS V
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

Map literals use `Map OF K TO V { ... }`:

```basic
LET ages = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
```

## 3. Built-in Functions

These functions are always in scope unless a package-qualified form is shown. Fallible functions return `Result` and therefore auto-propagate outside a direct `MATCH` scrutinee.

### 3.1 General

String length, search, substring, and regex indexes are zero-based Unicode scalar indexes, not byte offsets and not grapheme-cluster indexes. Use `strings::graphemes` when user-perceived character clusters are needed.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `len` | `FUNC len(value AS String) AS Integer` | Number of Unicode scalar values in `value`. |
| `len` | `FUNC len OF T(value AS List OF T) AS Integer` | Number of items in `value`. |
| `len` | `FUNC len OF K, V(value AS Map OF K TO V) AS Integer` | Number of entries in `value`. |
| `find` | `FUNC find(value AS String, needle AS String, start AS Integer = 0) AS Integer` | Zero-based scalar index of the first occurrence at or after `start`. Fails with `errorCode::ErrNotFound` (`77050004`) when absent and `77050001` when `start` is out of range. |
| `mid` | `FUNC mid(value AS String, start AS Integer, count AS Integer) AS String` | Returns a substring by zero-based Unicode scalar index. Fails with `77050001` on invalid range. |
| `replace` | `FUNC replace(value AS String, old AS String, new AS String) AS String` | Replaces all non-overlapping occurrences. |
| `typeName` | `FUNC typeName OF T(value AS T) AS String` | Implementation-defined display name of the static type. Intended for diagnostics. |
| `toString` | `FUNC toString(value AS Integer) AS String` | Converts an integer to base-10 text. |
| `toString` | `FUNC toString(value AS Float, precision AS Byte = 2) AS String` | Converts a float to implementation-defined round-trippable text. |
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

| Function | Signature | Behavior |
|----------|-----------|----------|
| `get` | `FUNC get OF T(value AS List OF T, index AS Integer) AS T` | Returns the item at zero-based `index`. Fails with `77050001` when out of range. |
| `get` | `FUNC get OF K, V(value AS Map OF K TO V, key AS K) AS V` | Returns the value for `key`. Fails with `errorCode::ErrNotFound` (`77050004`) when missing. |
| `getOr` | `FUNC getOr OF T(value AS List OF T, index AS Integer, default AS T) AS T` | Returns the indexed item or `default`. |
| `getOr` | `FUNC getOr OF K, V(value AS Map OF K TO V, key AS K, default AS V) AS V` | Returns the mapped value or `default`. |
| `find` | `FUNC find OF T(value AS List OF T, item AS T, start AS Integer = 0) AS Integer` | Zero-based index of the first matching item at or after `start`. `T` must be comparable. Fails with `errorCode::ErrNotFound` (`77050004`) when absent and `77050001` when `start` is out of range. |
| `find` | `FUNC find OF T(value AS List OF T, needle AS List OF T, start AS Integer = 0) AS Integer` | Zero-based index of the first contiguous `needle` sublist at or after `start`. `T` must be comparable. Fails with `errorCode::ErrNotFound` (`77050004`) when absent and `77050001` when `start` is out of range. |
| `mid` | `FUNC mid OF T(value AS List OF T, start AS Integer, count AS Integer) AS List OF T` | Returns a sublist by zero-based item index. Fails with `77050001` on invalid range. |
| `replace` | `FUNC replace OF T(value AS List OF T, old AS T, new AS T) AS List OF T` | Returns a list where every item equal to `old` is replaced with `new`. `T` must be comparable. |
| `set` | `FUNC set OF T(value AS List OF T, index AS Integer, item AS T) AS List OF T` | Returns a list with `item` at `index`. Fails with `77050001` when out of range. |
| `set` | `FUNC set OF K, V(value AS Map OF K TO V, key AS K, item AS V) AS Map OF K TO V` | Returns a map with `key` set to `item`. |
| `append` | `FUNC append OF T(value AS List OF T, item AS T) AS List OF T` | Returns a list with `item` added at the end. |
| `append` | `FUNC append OF T(value AS List OF T, items AS List OF T) AS List OF T` | Returns a list with all `items` added at the end. |
| `prepend` | `FUNC prepend OF T(value AS List OF T, item AS T) AS List OF T` | Returns a list with `item` added at the start. |
| `insert` | `FUNC insert OF T(value AS List OF T, index AS Integer, item AS T) AS List OF T` | Returns a list with `item` inserted before `index`. Fails with `77050001` when out of range. |
| `removeAt` | `FUNC removeAt OF T(value AS List OF T, index AS Integer) AS List OF T` | Returns a list without the item at `index`. Fails with `77050001` when out of range. |
| `removeKey` | `FUNC removeKey OF K, V(value AS Map OF K TO V, key AS K) AS Map OF K TO V` | Returns a map without `key`. Missing keys are ignored. |
| `keys` | `FUNC keys OF K, V(value AS Map OF K TO V) AS List OF K` | Returns the keys in implementation-defined stable order. |
| `values` | `FUNC values OF K, V(value AS Map OF K TO V) AS List OF V` | Returns the values in key iteration order. |
| `hasKey` | `FUNC hasKey OF K, V(value AS Map OF K TO V, key AS K) AS Boolean` | `TRUE` when `key` exists. |
| `contains` | `FUNC contains OF T(value AS List OF T, item AS T) AS Boolean` | `TRUE` when `item` appears in the list. `T` must be comparable. |
| `forEach` | `FUNC forEach OF T(value AS List OF T, action AS FUNC(T) AS Nothing) AS Nothing` | Calls `action` once for each item, left to right. A `SUB(T)` is accepted for `action`. |
| `transform` | `FUNC transform OF T, U(value AS List OF T, f AS FUNC(T) AS U) AS List OF U` | Maps each item through `f`. |
| `filter` | `FUNC filter OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS List OF T` | Keeps items where `predicate` returns `TRUE`. |
| `reduce` | `FUNC reduce OF T, U(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U` | Folds items left to right. |
| `sum` | `FUNC sum(value AS List OF Integer) AS Integer` | Sums integers. |
| `sum` | `FUNC sum(value AS List OF Float) AS Float` | Sums floats. |
| `sum` | `FUNC sum(value AS List OF Fixed) AS Fixed` | Sums fixed-point values. Fails with `77050010` on overflow. |

Collection callback parameters accept named functions, `SUB` values where `FUNC(... ) AS Nothing` is expected, and lambdas or closures that satisfy the language closure rules. Ordinary closures may capture only copyable `LET` bindings by value; capturing `MUT`, resource, or other non-copyable values is a compile-time error.

Ordinary `List` and `Map` values do not accept element, key, or value types that directly or transitively contain a resource handle or `Thread` handle. Ownership analysis rejects those collection instantiations before lowering.

When absence is expected, handle `find` with `MATCH`:

```basic
IMPORT errorCode

MATCH find(parts, "=")
  CASE Ok(i) : io::print("separator at " & toString(i))
  CASE Error(e) WHEN e.code = errorCode::ErrNotFound : io::print("separator not found")
  CASE Error(e) : FAIL e
END MATCH
```

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

## 6. Regex Package

Regular-expression helpers are exported by the `regex` package. Package functions are called with their package qualifier.

The regular-expression dialect is the Rust `regex` crate style supported by this implementation. It is compiler-defined and must behave the same across targets. Invalid patterns fail with `ErrInvalidFormat`.

Matching is Unicode-aware and user-visible indexes remain zero-based Unicode scalar indexes, not byte offsets. The implementation targets the Rust `regex` feature style rather than POSIX `regcomp()` semantics:

- syntax and matching behavior should follow Rust `regex` style
- backreferences and look-around are not supported
- behavior must not vary by target libc or OS regex library
- replacement behavior should follow Rust `regex`-style global replacement semantics supported by this implementation

| Function | Signature | Behavior |
|----------|-----------|----------|
| `regex::match` | `FUNC match(value AS String, pattern AS String) AS Boolean` | `TRUE` when `pattern` matches anywhere in `value`. |
| `regex::find` | `FUNC find(value AS String, pattern AS String, start AS Integer = 0) AS Integer` | Returns the zero-based scalar index of the first regex match at or after `start`. Fails with `ErrNotFound` when absent. |
| `regex::replace` | `FUNC replace(value AS String, pattern AS String, replacement AS String) AS String` | Replaces all regex matches. |

## 7. Built-in IO Package

Terminal and standard-stream I/O is provided by the `io` package. Package functions are called with their package qualifier.

```basic
TYPE TerminalSize
  columns AS Integer
  rows AS Integer
END TYPE
```

`TerminalSize` is a compiler-owned record shape returned by `io::terminalSize`.
It has public read-only fields and cannot be constructed or updated directly.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `io::print` | `FUNC print(value AS String) AS Nothing` | Writes `value` to standard output and appends a newline. Fails with `77020002` on output failure. |
| `io::write` | `FUNC write(value AS String) AS Nothing` | Writes `value` to standard output without appending a newline. Fails with `77020002` on output failure. |
| `io::printError` | `FUNC printError(value AS String) AS Nothing` | Writes `value` to standard error and appends a newline. Fails with `77020002` on output failure. |
| `io::writeError` | `FUNC writeError(value AS String) AS Nothing` | Writes `value` to standard error without appending a newline. Fails with `77020002` on output failure. |
| `io::flush` | `FUNC flush() AS Nothing` | Flushes standard output. Fails with `77020002` on output failure. |
| `io::flushError` | `FUNC flushError() AS Nothing` | Flushes standard error. Fails with `77020002` on output failure. |
| `io::input` | `FUNC input(prompt AS String = "") AS String` | Writes `prompt` to standard output when non-empty, flushes standard output, reads one line from standard input, and returns it without the line terminator. Fails with `77020003` at EOF, `77020004` on invalid UTF-8 input, and `77020005` on input failure. |
| `io::readLine` | `FUNC readLine() AS String` | Reads one line from standard input and returns it without the line terminator. Fails with `77020003` at EOF, `77020004` on invalid UTF-8 input, and `77020005` on input failure. |
| `io::readChar` | `FUNC readChar() AS String` | Reads one Unicode scalar value from standard input and returns it as a `String`. Fails with `77020003` at EOF, `77020004` on invalid UTF-8, and `77020005` on input failure. |
| `io::readByte` | `FUNC readByte() AS Byte` | Reads one byte from standard input. Fails with `77020003` at EOF and `77020005` on input failure. |
| `io::pollInput` | `FUNC pollInput(timeoutMs AS Integer = 0) AS Boolean` | Waits until standard input can be read without blocking. `timeoutMs < 0` waits forever, `timeoutMs = 0` performs a nonblocking readiness check, and `timeoutMs > 0` waits up to that many milliseconds. Returns `TRUE` when input is ready and `FALSE` on timeout. Fails with `77020005` on input polling failure. |
| `io::isInputTerminal` | `FUNC isInputTerminal() AS Boolean` | `TRUE` when standard input is attached to an interactive terminal. |
| `io::isOutputTerminal` | `FUNC isOutputTerminal() AS Boolean` | `TRUE` when standard output is attached to an interactive terminal. |
| `io::isErrorTerminal` | `FUNC isErrorTerminal() AS Boolean` | `TRUE` when standard error is attached to an interactive terminal. |
| `io::terminalSize` | `FUNC terminalSize() AS TerminalSize` | Returns the current interactive terminal size for standard output. Fails with `77050007` when standard output is not an interactive terminal or the host cannot report a size. |

Standard input character reads use the host terminal's normal line discipline. On canonical terminals, `io::readChar` may not return until the user submits a line; raw keypress mode, nonblocking input, cursor control, colors, and alternate-screen behavior are outside the core `io` package and may be provided by a future terminal package.

There is no `PRINT` statement and no trailing-semicolon newline suppression. Use `io::print` for newline-terminated standard output, `io::write` for standard output without a newline, `io::printError` or `io::writeError` for standard error, and `fs::writeAll` for file-handle output.

Use `toString` explicitly before calling `io::print`, `io::write`, `io::printError`, or `io::writeError` when outputting a non-string value. Output functions are intended for user-visible text and diagnostics, not automatic structured logging of arbitrary values.

## 8. Built-in Filesystem Package

Filesystem and file-handle functions live in the `fs` package. Paths are `String` values.

One-shot path operations read, write, inspect, or modify filesystem entries without exposing resource handles. Scoped file-handle I/O uses `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, or `fs::createTempFile` with `USING`; the resulting `File` is closed automatically at `END USING`.

```basic
USING file = fs::open("data.txt", "read")
  ' file is in scope here
END USING
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
| `fs::open` | `FUNC open(path AS String, mode AS String) AS File` | Opens a file handle for use with `USING`. Portable modes are `"read"`/`"r"`, `"write"`/`"w"`, `"readWrite"`/`"rw"`, and `"append"`/`"a"`. Invalid modes, empty paths, and embedded NUL bytes fail with `ErrInvalidArgument` (`77050002`). Missing files fail with `ErrNotFound` (`77050004`) for read-style opens. |
| `fs::openFile` | `FUNC openFile(path AS String, mode AS String = "read") AS File` | Opens a file handle. `mode` is `"read"`, `"write"`, or `"append"`. Fails with `77030001`, `77030002`, or `77030003`. |
| `fs::openFileNoFollow` | `FUNC openFileNoFollow(path AS String, mode AS String = "read") AS File` | Opens a file handle like `fs::openFile` but fails with `ErrAccessDenied` when the final path component is a symlink. |
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
| `fs::listDirectory` | `FUNC listDirectory(path AS String) AS List OF String` | Lists direct child names in implementation-defined stable order. |
| `fs::currentDirectory` | `FUNC currentDirectory() AS String` | Returns the current working directory. |
| `fs::setCurrentDirectory` | `FUNC setCurrentDirectory(path AS String) AS Nothing` | Changes the current working directory. |

`File` is an opaque standard `RESOURCE` type and unique handle. It can be bound by `USING` and is closed automatically with `fs::close` at `END USING`.

## 9. Built-in Thread Package

Thread functions live in the `thread` package. Thread entry points must be exported `ISOLATED FUNC` declarations from imported packages with type `ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out`; lambdas, closures, `SUB`s, non-isolated functions, current-package functions, and functions without the leading worker handle parameter are rejected at compile time.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `thread::start` | `FUNC start OF In, Msg, Out(f AS ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out, data AS In, inboundLimit AS Integer = 64, outboundLimit AS Integer = 64) AS Thread OF Msg TO Out` | Starts imported package export `f` in a fresh package instance with bounded inbound and outbound queues, passing the worker-side thread handle and `data` into the thread by copy, move, or freeze. Current-package functions are rejected at compile time. Fails with `77050002` for queue limits below `1`. |
| `thread::isRunning` | `FUNC isRunning OF Msg, Out(t AS Thread OF Msg TO Out) AS Boolean` | `TRUE` while the thread entry function is still running. |
| `thread::waitFor` | `FUNC waitFor OF Msg, Out(t AS Thread OF Msg TO Out) AS Out` | Waits for completion, then returns the thread's stored result. `Err` auto-propagates like any fallible function. |
| `thread::cancel` | `FUNC cancel OF Msg, Out(t AS Thread OF Msg TO Out) AS Nothing` | Requests cooperative cancellation. New sends fail after cancellation is requested. |
| `thread::send` | `FUNC send OF Msg, Out(t AS Thread OF Msg TO Out, data AS Msg, timeoutMs AS Integer = 0) AS Nothing` | Sends `data` to the thread's inbound queue by copy, move, or freeze. `timeoutMs = 0` does not wait for queue space. Fails if the thread has ended, cancellation was requested, the timeout expires, or the timeout is invalid. |
| `thread::send` | `FUNC send OF Msg, Out(t AS ThreadWorker OF Msg TO Out, data AS Msg, timeoutMs AS Integer = 0) AS Nothing` | Sends `data` from the worker to the parent-visible outbound queue by copy, move, or freeze. `timeoutMs = 0` does not wait for queue space. Fails if the thread has ended, the timeout expires, or the timeout is invalid. |
| `thread::poll` | `FUNC poll OF Msg, Out(t AS Thread OF Msg TO Out, ms AS Integer) AS Boolean` | Waits up to `ms` milliseconds for an outbound message. Returns `TRUE` when `thread::receive(t)` can read without blocking. |
| `thread::receive` | `FUNC receive OF Msg, Out(t AS Thread OF Msg TO Out, timeoutMs AS Integer = 0) AS Msg` | Parent-side read of the next outbound message by copy, move, or freeze. `timeoutMs = 0` does not wait for a message. Fails when no message is available before the timeout. |
| `thread::receive` | `FUNC receive OF Msg, Out(t AS ThreadWorker OF Msg TO Out, timeoutMs AS Integer = 0) AS Msg` | Worker-side read from the inbound queue for `t`. Valid only inside the worker thread. `timeoutMs = 0` does not wait. |
| `thread::isCancelled` | `FUNC isCancelled OF Msg, Out(t AS ThreadWorker OF Msg TO Out) AS Boolean` | Worker-side cancellation check. Returns `TRUE` after the parent requests cancellation. |

When a thread ends, its `Thread` value keeps `result AS Result OF Out`. `thread::waitFor(t)` waits for that result to exist and returns it with normal `Result` auto-unwrapping behavior.

Thread functions are ordinary built-in templates. Their `Msg` and `Out` parameters are resolved by the template rules in the language specification from argument types and expected result types. `thread::start` gets `Msg` and `Out` from the started function's first `ThreadWorker OF Msg TO Out` parameter, and gets `In` from the started function's second parameter and the `data` argument. If a thread does not exchange messages, `Msg` may be `Nothing`.

`Thread` is the parent-side handle and `ThreadWorker` is the worker-side handle. `thread::send` and `thread::receive` are overloaded on those handle types to select the inbound or outbound queue. Queue timeout uses `ErrTimeout`; unavailable messages use `ErrNotFound`; cancellation uses `ErrInterrupted`.

## 10. Built-in Math Package

Math functions live in the `math` package. Constants are `LET` values and must be referenced as identifiers, not called like zero-argument functions. Numeric functions are overloaded by argument type; mixed numeric calls require an explicit conversion.

Math functions follow the numeric edge-case rules in §4.1. Integer and `Fixed` overflow fails with `ErrOverflow` (`77050010`). Invalid domains, such as square root of a negative value or logarithm of a non-positive value, fail with `ErrInvalidArgument` (`77050002`). `Float` functions return only finite values; a result that would be NaN is an invalid-domain error, and a result that would be infinity is an overflow error.

| Constant          | Type    | Value |
|-------------------|---------|-------|
| `math::pi`        | `Float` | The mathematical constant pi as a `Float`. |
| `math::piFixed`   | `Fixed` | The mathematical constant pi rounded to the nearest `Fixed` value. |
| `math::2pi`       | `Float` | The mathematical constant 2 / pi as a `Float`. |
| `math::2piFixed`  | `Fixed` | The mathematical constant 2 / pi rounded to the nearest `Fixed` value. |
| `math::pi2`       | `Float` | The mathematical constant pi / 2 as a `Float`. |
| `math::pi2Fixed`  | `Fixed` | The mathematical constant pi / 2 rounded to the nearest `Fixed` value. |
| `math::pi4`       | `Float` | The mathematical constant pi / 4 as a `Float`. |
| `math::pi4Fixed`  | `Fixed` | The mathematical constant pi / 4 rounded to the nearest `Fixed` value. |
| `math::e`         | `Float` | The mathematical constant e as a `Float`. |
| `math::eFixed`    | `Fixed` | The mathematical constant e rounded to the nearest `Fixed` value. |
| `math::ln2`       | `Float` | The mathematical constant ln(2) as a `Float`. |
| `math::ln2Fixed`  | `Fixed` | The mathematical constant ln(2) rounded to the nearest `Fixed` value. |
| `math::ln10`      | `Float` | The mathematical constant ln(10) as a `Float`. |
| `math::ln10Fixed` | `Fixed` | The mathematical constant ln(10) rounded to the nearest `Fixed` value. |

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
| `math::sqrt` | `FUNC sqrt(value AS Float) AS Float` | Square root. Fails with `77050002` for negative input. |
| `math::sqrt` | `FUNC sqrt(value AS Fixed) AS Fixed` | Fixed-point square root rounded to nearest `Fixed`. Fails with `77050002` for negative input. |
| `math::pow` | `FUNC pow(base AS Float, exponent AS Float) AS Float` | Power function. |
| `math::pow` | `FUNC pow(base AS Fixed, exponent AS Fixed) AS Fixed` | Fixed-point power rounded to nearest `Fixed`. Fails with `77050002` for invalid domains and `77050010` on overflow. |
| `math::exp` | `FUNC exp(value AS Float) AS Float` | e raised to `value`. |
| `math::exp` | `FUNC exp(value AS Fixed) AS Fixed` | Fixed-point e raised to `value`, rounded to nearest `Fixed`. Fails with `77050010` on overflow. |
| `math::log` | `FUNC log(value AS Float) AS Float` | Natural logarithm. Fails with `77050002` for non-positive input. |
| `math::log` | `FUNC log(value AS Fixed) AS Fixed` | Fixed-point natural logarithm rounded to nearest `Fixed`. Fails with `77050002` for non-positive input. |
| `math::log10` | `FUNC log10(value AS Float) AS Float` | Base-10 logarithm. Fails with `77050002` for non-positive input. |
| `math::log10` | `FUNC log10(value AS Fixed) AS Fixed` | Fixed-point base-10 logarithm rounded to nearest `Fixed`. Fails with `77050002` for non-positive input. |
| `math::sin` | `FUNC sin(value AS Float) AS Float` | Sine, radians. |
| `math::sin` | `FUNC sin(value AS Fixed) AS Fixed` | Fixed-point sine, radians, rounded to nearest `Fixed`. |
| `math::cos` | `FUNC cos(value AS Float) AS Float` | Cosine, radians. |
| `math::cos` | `FUNC cos(value AS Fixed) AS Fixed` | Fixed-point cosine, radians, rounded to nearest `Fixed`. |
| `math::tan` | `FUNC tan(value AS Float) AS Float` | Tangent, radians. |
| `math::tan` | `FUNC tan(value AS Fixed) AS Fixed` | Fixed-point tangent, radians, rounded to nearest `Fixed`. Fails with `77050002` at undefined points. |
| `math::asin` | `FUNC asin(value AS Float) AS Float` | Arc sine. Fails with `77050002` when outside `[-1.0, 1.0]`. |
| `math::asin` | `FUNC asin(value AS Fixed) AS Fixed` | Fixed-point arc sine rounded to nearest `Fixed`. Fails with `77050002` when outside `[-1.0, 1.0]`. |
| `math::acos` | `FUNC acos(value AS Float) AS Float` | Arc cosine. Fails with `77050002` when outside `[-1.0, 1.0]`. |
| `math::acos` | `FUNC acos(value AS Fixed) AS Fixed` | Fixed-point arc cosine rounded to nearest `Fixed`. Fails with `77050002` when outside `[-1.0, 1.0]`. |
| `math::atan` | `FUNC atan(value AS Float) AS Float` | Arc tangent. |
| `math::atan` | `FUNC atan(value AS Fixed) AS Fixed` | Fixed-point arc tangent rounded to nearest `Fixed`. |
| `math::atan2` | `FUNC atan2(y AS Float, x AS Float) AS Float` | Two-argument arc tangent using the standard `atan2(y, x)` convention. |
| `math::atan2` | `FUNC atan2(y AS Fixed, x AS Fixed) AS Fixed` | Fixed-point two-argument arc tangent using the standard `atan2(y, x)` convention, rounded to nearest `Fixed`. |

## 11. Built-in Net Package

Network functions live in the `net` package. Socket handles are opaque standard `RESOURCE` types and unique handles. They can be bound by `USING`; `net::close` runs automatically at `END USING`.

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
| Connect/open handshake | `net::connectTcp`, `tls::connect`, `tls::wrap` | Use the implementation default timeout. |
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
| `net::poll` | `FUNC poll(sock AS List OF Socket, timeoutMs AS Integer = 0) AS List OF Boolean` | Returns booleans aligned with `sock`; each item is `TRUE` when the socket at the same index can be read without blocking before `timeoutMs` expires. `timeoutMs = 0` polls without waiting. Fails with `77050002` or `77030004`. |
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
IMPORT net

LET addresses = net::lookup("example.com", 80)
LET address = get(addresses, 0)

USING client = net::connectTcp(address, timeoutMs := 5000)
  net::writeText(client, "ping")
  LET chunk = net::readText(client, 4096)
  io::print(chunk)
END USING
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
| `tls::connect` | `FUNC connect(host AS String, port AS Integer, timeoutMs AS Integer = 0, serverName AS String = "") AS TlsSocket` | Opens a TCP connection and performs a TLS client handshake. Empty `serverName` means use `host` for certificate validation. `timeoutMs = 0` uses the implementation default. |
| `tls::wrap` | `FUNC wrap(sock AS Socket, serverName AS String, timeoutMs AS Integer = 0) AS TlsSocket` | Consumes a connected TCP `Socket` and performs a TLS client handshake over it. The plain socket must not be used afterward. `timeoutMs = 0` uses the implementation default. |
| `tls::read` | `FUNC read(sock AS TlsSocket, maxBytes AS Integer) AS List OF Byte` | Reads decrypted bytes. Fails with the same read errors as `net::read`, plus TLS validation or protocol errors. |
| `tls::readText` | `FUNC readText(sock AS TlsSocket, maxBytes AS Integer) AS String` | Reads decrypted bytes and decodes UTF-8 text. |
| `tls::write` | `FUNC write(sock AS TlsSocket, bytes AS List OF Byte) AS Nothing` | Encrypts and writes all bytes before returning. |
| `tls::writeText` | `FUNC writeText(sock AS TlsSocket, value AS String) AS Nothing` | Encodes `value` as UTF-8, encrypts it, and writes all bytes. |
| `tls::close` | `FUNC close(resource AS TlsSocket) AS Nothing` | Closes the TLS session and underlying transport. Calling it more than once is an error. |

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

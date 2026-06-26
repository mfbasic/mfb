# 18. Built-in Functions

There are exactly **two scoping tiers** of built-in. This section is about the
**language-level always-in-scope** tier; the import-gated standard packages are
catalogued only for orientation (their full surface lives in their own
documentation, e.g. `mfb man <package>`).

## 18.1 Always-in-scope general built-ins

These **sixteen** names are the *only* callables a program may use with no
`IMPORT` and no package qualifier. The resolver treats exactly this set as
always-in-scope unqualified callables (`src/resolver.rs` `resolve_callable`,
which whitelists `builtins::general::is_general_call`); the set is defined in
`src/builtins/general.rs` (`is_general_call`, lines 40–60):

| Name | Arity | Result | Accepted argument types |
| --- | --- | --- | --- |
| `error(code, message)` | 2 | `Error` | `(Integer, String)` — builds the read-only `Error` record |
| `len(value)` | 1 | `Integer` | `String`, `List OF T`, `Map OF K TO V` |
| `typeName(value)` | 1 | `String` | any `T` (never reads the value) |
| `toString(value[, decimals])` | 1–2 | `String` | `Integer`/`Float`/`Fixed`/`Boolean`/`String`/`Byte`/`List OF Byte`; optional `Byte` precision for `Float`/`Fixed` |
| `toInt(value)` | 1 | `Integer` | `String`, `Byte`, `Float`, `Fixed` |
| `toFloat(value)` | 1 | `Float` | `String`, `Integer`, `Fixed` |
| `toFixed(value)` | 1 | `Fixed` | `String`, `Integer`, `Float` |
| `toByte(value)` | 1 | `Byte` | `Integer` |
| `isNumeric(value)` | 1 | `Boolean` | `String` |
| `isEven(value)` / `isOdd(value)` | 1 | `Boolean` | `Integer` |
| `isPositive` / `isNegative` / `isZero` | 1 | `Boolean` | `Integer`, `Float`, `Fixed` |
| `isEmpty(value)` / `isNotEmpty(value)` | 1 | `Boolean` | `String`, `List OF T`, `Map OF K TO V` |

> The conversion name is `toInt` (not `toInteger`). String concatenation `&` is a
> binary **operator** (`src/lexer.rs` `TokenKind::Ampersand`, lowered in
> `src/typecheck.rs`), not a built-in function — it is not in this set.
>
> The `is*` predicates are **inlined** builtins (`src/builtins/general.rs`
> `builtin_function_id`); they cannot be passed as a function value directly.
> Wrap one in a `FUNC`/`LAMBDA` where a predicate argument is required.

All sixteen except `error` are **overridable** (see §18.3). Every other built-in
member named below lives in an **import-gated standard package** and is *not*
in scope without its `IMPORT`. The package set the resolver recognizes is fixed
(`src/builtins/mod.rs` `is_builtin_import`): `collections`, `csv`, `datetime`,
`errorCode`, `fs`, `http`, `io`, `json`, `math`, `net`, `regex`, `strings`,
`term`, `thread`, `tls`. A bare unqualified `find`, `get`, `append`, `print`,
… is a `SYMBOL_UNKNOWN_IDENTIFIER` error; a qualified `io::print` without
`IMPORT io` is a `SYMBOL_UNKNOWN_IMPORT` error.

## 18.2 Import-gated standard packages (orientation only)

Each row requires the named `IMPORT`. (`find`/`mid`/`replace` and the
collection accessors share resolver logic in `src/builtins/general.rs` but are
reached only through their `strings::`/`collections::` qualifiers — never as
bare names.)

Terminal and standard-stream I/O (`IMPORT io`): `io::print`, `io::write`, `io::printError`, `io::writeError`, `io::flush`, `io::flushError`, `io::input`, `io::readLine`, `io::readChar`, `io::readByte`, `io::isInputTerminal`, `io::isOutputTerminal`, `io::isErrorTerminal`.
Structured terminal / TUI control (`IMPORT term`): `term::on`, `term::off`, `term::isOn`, `term::setForeground`, `term::setBackground`, `term::setBold`, `term::setUnderline`, `term::showCursor`, `term::hideCursor`, `term::clear`, `term::moveTo`, `term::getForeground`, `term::getBackground`, `term::getBold`, `term::getUnderline`, `term::terminalSize`.
Filesystem and file I/O (`IMPORT fs`): `fs::fileExists`, `fs::directoryExists`, `fs::exists`, `fs::readBytes`, `fs::readText`, `fs::writeBytes`, `fs::writeText`, `fs::writeBytesAtomic`, `fs::writeTextAtomic`, `fs::appendBytes`, `fs::appendText`, `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, `fs::createTempFile`, `fs::tempDirectory`, `fs::readLine`, `fs::readAll`, `fs::readAllBytes`, `fs::writeAll`, `fs::writeAllBytes`, `fs::close`, `fs::eof`, `fs::canonicalPath`, `fs::isWithin`, `fs::pathJoin`, `fs::pathDirName`, `fs::pathBaseName`, `fs::pathExtension`, `fs::pathNormalize`, `fs::deleteFile`, `fs::createDirectory`, `fs::createDirectories`, `fs::deleteDirectory`, `fs::listDirectory`, `fs::currentDirectory`, `fs::setCurrentDirectory`.
Network (`IMPORT net`, `IMPORT tls`): `net::lookup`, `net::connectTcp`, `net::listenTcp`, `net::accept`, `net::bindUdp`, `net::receiveFrom`, `net::receiveTextFrom`, `net::sendTo`, `net::sendTextTo`, `net::poll`, `net::read`, `net::readText`, `net::write`, `net::writeText`, `net::close`, `net::localAddress`, `net::remoteAddress`, `net::setReadTimeout`, `net::setWriteTimeout`, `tls::connect`, `tls::wrap`, `tls::close`.
Strings (`IMPORT strings`): `strings::find`, `strings::mid`, `strings::replace`, `strings::trim`, `strings::trimStart`, `strings::trimEnd`, `strings::upper`, `strings::lower`, `strings::caseFold`, `strings::normalizeNfc`, `strings::graphemes`, `strings::startsWith`, `strings::endsWith`, `strings::contains`, `strings::split`, `strings::join`, `strings::byteLen`. (`len`, `toString`, `toInt`, `toFloat`, `toFixed`, `toByte`, `isNumeric` are general always-in-scope built-ins, §18.1; `&` is the concatenation operator.)
Regex (`IMPORT regex`): `regex::match`, `regex::find`, `regex::findAll`, `regex::replace`.
Collections (`IMPORT collections`): `collections::forEach`, `collections::transform`, `collections::filter`, `collections::reduce`, `collections::sum`, `collections::get`, `collections::getOr`, `collections::find`, `collections::mid`, `collections::replace`, `collections::set`, `collections::append`, `collections::prepend`, `collections::insert`, `collections::removeAt`, `collections::removeKey`, `collections::keys`, `collections::values`, `collections::hasKey`, `collections::contains`. (`len` of a `List`/`Map` is the general built-in, §18.1.)
Threads (`IMPORT thread`): `thread::start`, `thread::isRunning`, `thread::waitFor`, `thread::cancel`, `thread::send`, `thread::poll`, `thread::receive`, `thread::isCancelled`.
Math (`IMPORT math`): `math::pi`, `math::piFixed`, `math::e`, `math::eFixed`, `math::abs`, `math::min`, `math::max`, `math::clamp`, `math::floor`, `math::ceil`, `math::round`, `math::sqrt`, `math::pow`, `math::exp`, `math::log`, `math::log10`, `math::sin`, `math::cos`, `math::tan`, `math::asin`, `math::acos`, `math::atan`, `math::atan2`.
JSON (`IMPORT json`): `json::parse`, `json::stringify`, `json::get`, `json::getOr`.
Error codes (`IMPORT errorCode`): `errorCode::ErrInvalidArgument`, `errorCode::ErrNotFound`, and the other constants listed in the built-in error-code registry. These are compile-time `Integer` constants (`src/builtins/errorcode.rs`), not callables.

> CSV (`IMPORT csv`), HTTP (`IMPORT http`), and datetime (`IMPORT datetime`)
> are additional import-gated packages; see their own documentation.

Fallible built-ins (`fs::openFile`, `toInt`, `collections::get`, …) can fail and auto-propagate like any call.

## 18.3 Overriding general built-ins

The **general (unqualified) built-ins** — `toString`, `len`, `typeName`, the `to*` conversions (`toInt`, `toFloat`, `toFixed`, `toByte`), and the `is*` predicates (`isNumeric`, `isEven`, `isOdd`, `isPositive`, `isNegative`, `isZero`, `isEmpty`, `isNotEmpty`) — are **overridable**: a program or package may declare, e.g., `FUNC toString(value AS Point) AS String` or `FUNC len(value AS Grid) AS Integer`, and a plain `toString(p)` / `len(g)` call binds to that declaration when its argument types match. Resolution is **gap-fill**: the scalar/collection built-in stays authoritative for the types it already supports (a user overload can never shadow `toString(42)`), and an override is consulted only when the built-in rejects the argument types. The override is selected by argument type like any overload. `error` is **not** overridable — it is a reserved primitive that builds the read-only `Error` record (`FUNC error(…)` is a `SYMBOL_RESERVED_BUILTIN_NAME` error).

Implementation: overridability is `src/builtins/general.rs` `is_overridable`
(every general name except `error`); the gap-fill routing is
`src/monomorph.rs` `resolve_general_builtin_override`, which consults a user
override **only** when `builtins::general::resolve_call` rejects the argument
types, so a user overload can never shadow a type the built-in already handles.
The reserved check is `reserved_builtin_name` (the set is exactly `{ error }`),
enforced in `src/resolver.rs` `insert_function`.

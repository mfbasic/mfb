# 18. Built-in Functions

There are exactly **two scoping tiers** of built-in. This section is about the
**language-level always-in-scope** tier; the import-gated standard packages are
catalogued only for orientation (their full surface lives in their own
documentation, e.g. `mfb man <package>`).

## 18.1 Always-in-scope general built-ins

These **sixteen** names are the *only* callables a program may use with no
`IMPORT` and no package qualifier. The resolver treats exactly this set as
always-in-scope unqualified callables (`resolve_callable` in `src/resolver/resolution.rs`,
which whitelists `builtins::general::is_general_call`); the set is defined in
`src/builtins/general.rs` (`is_general_call`, lines 40–60):

| Name | Arity | Result | Accepted argument types |
| --- | --- | --- | --- |
| `error(code, message)` | 2 | `Error` | `(Integer, String)` — builds the read-only `Error` record |
| `len(value)` | 1 | `Integer` | `String`, `List OF T`, `Map OF K TO V` |
| `typeName(value)` | 1 | `String` | any `T` (never reads the value) |
| `toString(value[, decimals])` | 1–2 | `String` | `Integer`/`Float`/`Fixed`/`Boolean`/`String`/`Byte`/`List OF Byte`; optional `Byte` precision for `Float`/`Fixed` |
| `toInt(value[, base])` | 1–2 | `Integer` | `String`, `Byte`, `Float`, `Fixed`; optional `Integer` `base` (2–36) for `String` radix parsing |
| `toFloat(value)` | 1 | `Float` | `String`, `Integer`, `Fixed` |
| `toFixed(value)` | 1 | `Fixed` | `String`, `Integer`, `Float` |
| `toByte(value)` | 1 | `Byte` | `Integer` |
| `isNumeric(value)` | 1 | `Boolean` | `String` |
| `isEven(value)` / `isOdd(value)` | 1 | `Boolean` | `Integer` |
| `isPositive` / `isNegative` / `isZero` | 1 | `Boolean` | `Integer`, `Float`, `Fixed` |
| `isEmpty(value)` / `isNotEmpty(value)` | 1 | `Boolean` | `String`, `List OF T`, `Map OF K TO V` |

> The conversion name is `toInt` (not `toInteger`). String concatenation `&` is a
> binary **operator** (`src/lexer.rs` `TokenKind::Ampersand`), not a built-in
> function — it is not in this set.
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
bare names.) The member lists below are **non-exhaustive orientation
snapshots**: each package's authoritative surface is the corresponding
`is_<pkg>_call` matcher in `src/builtins/<pkg>.rs` (and the rendered `mfb man
<package>` pages). Where a package's full set is large, only a representative
subset is shown.

Terminal and standard-stream I/O (`IMPORT io`): `io::print`, `io::write`, `io::printError`, `io::writeError`, `io::flush`, `io::isBuffered`, `io::setBuffered`, `io::input`, `io::readLine`, `io::readChar`, `io::readByte`, `io::pollInput`, `io::isInputTerminal`, `io::isOutputTerminal`, `io::isErrorTerminal`. (`src/builtins/io.rs` `is_io_call`.)
Structured terminal / TUI control (`IMPORT term`): `term::on`, `term::off`, `term::isOn`, `term::setForeground`, `term::setBackground`, `term::setBold`, `term::setUnderline`, `term::showCursor`, `term::hideCursor`, `term::clear`, `term::moveTo`, `term::getForeground`, `term::getBackground`, `term::getBold`, `term::getUnderline`, `term::terminalSize`.
Filesystem and file I/O (`IMPORT fs`): `fs::fileExists`, `fs::directoryExists`, `fs::exists`, `fs::readBytes`, `fs::readText`, `fs::writeBytes`, `fs::writeText`, `fs::writeBytesAtomic`, `fs::writeTextAtomic`, `fs::appendBytes`, `fs::appendText`, `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, `fs::createTempFile`, `fs::tempDirectory`, `fs::readLine`, `fs::readAll`, `fs::readAllBytes`, `fs::writeAll`, `fs::writeAllBytes`, `fs::setBuffered`, `fs::isBuffered`, `fs::flush`, `fs::close`, `fs::eof`, `fs::canonicalPath`, `fs::isWithin`, `fs::pathJoin`, `fs::pathDirName`, `fs::pathBaseName`, `fs::pathExtension`, `fs::pathNormalize`, `fs::deleteFile`, `fs::createDirectory`, `fs::createDirectories`, `fs::deleteDirectory`, `fs::listDirectory`, `fs::currentDirectory`, `fs::setCurrentDirectory`.
Network (`IMPORT net`, `IMPORT tls`): `net::lookup`, `net::connectTcp`, `net::listenTcp`, `net::accept`, `net::bindUdp`, `net::receiveFrom`, `net::receiveTextFrom`, `net::sendTo`, `net::sendTextTo`, `net::poll`, `net::read`, `net::readText`, `net::write`, `net::writeText`, `net::close`, `net::localAddress`, `net::remoteAddress`, `net::setReadTimeout`, `net::setWriteTimeout`, `net::toUrl` (`src/builtins/net.rs` `is_net_call`); `tls::connect`, `tls::listen`, `tls::accept`, `tls::read`, `tls::readText`, `tls::write`, `tls::writeText`, `tls::close` (`src/builtins/tls.rs` `is_tls_call`; the additional `tls::closeListener` member is an internal listener-close dispatch target, not a user-callable surface — `tls::close` over a `TlsListener` rewrites to it during IR lowering).
Strings (`IMPORT strings`, representative subset — full set in `src/builtins/strings.rs` `is_strings_call`): `strings::find`, `strings::mid`, `strings::replace`, `strings::trim`, `strings::trimStart`, `strings::trimEnd`, `strings::trimChars`, `strings::upper`, `strings::lower`, `strings::caseFold`, `strings::normalizeNfc`, `strings::graphemes`, `strings::graphemeAt`, `strings::graphemesCount`, `strings::startsWith`, `strings::endsWith`, `strings::startsWithAny`, `strings::endsWithAny`, `strings::stripPrefix`, `strings::stripSuffix`, `strings::contains`, `strings::count`, `strings::left`, `strings::right`, `strings::repeat`, `strings::padLeft`, `strings::padRight`, `strings::split`, `strings::join`, `strings::byteLen`. (`len`, `toString`, `toInt`, `toFloat`, `toFixed`, `toByte`, `isNumeric` are general always-in-scope built-ins, §18.1; `&` is the concatenation operator.)
Regex (`IMPORT regex`): `regex::match`, `regex::find`, `regex::findAll`, `regex::replace`.
Collections (`IMPORT collections`): the migrated native accessors `collections::forEach`, `collections::transform`, `collections::filter`, `collections::reduce`, `collections::sum`, `collections::get`, `collections::getOr`, `collections::find`, `collections::mid`, `collections::replace`, `collections::set`, `collections::append`, `collections::prepend`, `collections::insert`, `collections::removeAt`, `collections::removeKey`, `collections::keys`, `collections::values`, `collections::hasKey`, `collections::contains` (`src/builtins/collections.rs` `NATIVE_MEMBERS`), plus the MFBASIC-source generics `collections::sort`, `sortBy`, `take`, `drop`, `reduceRight`, `any`, `all`, `findIndex`, `findLastIndex`, `groupBy`, `mapValues`, `flatten`, `zip`, `chunks`, `window`, `distinct`, `merge`, `partition` (`FUNCTIONS`). (`len` of a `List`/`Map` is the general built-in, §18.1.)
Threads (`IMPORT thread`, `src/builtins/thread.rs` `is_thread_call`): `thread::start`, `thread::isRunning`, `thread::waitFor`, `thread::cancel`, `thread::send`, `thread::poll`, `thread::receive`, `thread::isCancelled`, and the resource/value transfer-plane members `thread::transfer`, `thread::accept`, `thread::transferResource`, `thread::acceptResource`, `thread::emitResource`, `thread::readResource`.
Math (`IMPORT math`): the call members `math::abs`, `math::min`, `math::max`, `math::clamp`, `math::floor`, `math::ceil`, `math::round`, `math::sqrt`, `math::pow`, `math::exp`, `math::log`, `math::log10`, `math::sin`, `math::cos`, `math::tan`, `math::asin`, `math::acos`, `math::atan`, `math::atan2`, `math::rand`, `math::seed` (`src/builtins/math.rs` `is_math_call`), and the compile-time constants `math::pi`, `math::piFixed`, `math::e`, `math::eFixed` (`is_math_constant`, fold to literals like the `errorCode::Err*` registry — not callables).

**Array (SIMD) overloads.** Most `math::` members also accept a homogeneous numeric **list** and return a freshly allocated list, computing every element with AArch64 NEON vector instructions (two 64-bit lanes per instruction; `mfb spec architecture aarch64-instruction-set` "NEON vector ops"). Selection is by argument type (a `List OF …` argument picks the array overload):

| Member | Array overload(s) | Per-lane error |
|---|---|---|
| `abs` | `Integer[]→Integer[]`, `Fixed[]→Fixed[]`, `Float[]→Float[]` | `ErrOverflow` (Integer/Fixed min value) |
| `floor`/`ceil`/`round` | `Float[]→Integer[]`, `Fixed[]→Integer[]` | `ErrOverflow` (Float out of `Integer` range) |
| `min`/`max` | `(T[],T[])→T[]` for `T∈{Integer,Float,Fixed}` | `ErrInvalidArgument` (lengths differ) |
| `clamp` | `(T[],T,T)→T[]` for `T∈{Integer,Float,Fixed}` | `ErrInvalidArgument` (low > high) |
| `sqrt` | `Float[]→Float[]`, `Fixed[]→Fixed[]` | negative lane → `ErrFloatDomain` (Float) / `ErrInvalidArgument` (Fixed) |
| `log`/`log10` | `Float[]→Float[]`, `Fixed[]→Fixed[]` | lane ≤ 0 → `ErrFloatDomain` (Float) / `ErrInvalidArgument` (Fixed) |
| `exp` | `Float[]→Float[]` | `ErrFloatInf` (overflow), `ErrFloatNan` (NaN input) |
| `sin`/`cos`/`tan`/`atan` | `Float[]→Float[]` | `ErrFloatNan` (NaN result) |
| `asin`/`acos` | `Float[]→Float[]` | lane outside `[-1,1]` → `ErrFloatDomain` |
| `pow`/`atan2` | `(Float[],Float[])→Float[]` | `ErrInvalidArgument` (lengths differ), `ErrFloatNan` (NaN result) |

The per-lane error codes deliberately match the scalar `math::` overloads (the
`mfb man math` pages): the `Float` overloads use the float-specific
`ErrFloatDomain`/`ErrFloatInf`/`ErrFloatNan`, and `Fixed` uses `ErrInvalidArgument`,
so `math::f(x)` and `math::f([x])[0]` fail identically.

A per-lane error is reported as a single error **after** processing all lanes (the result list is discarded), so the error is deterministic regardless of which lane failed. `Fixed[]` results are platform-independent (deterministic Q32.32; `sqrt` is a real 2-lane NEON kernel, `log`/`log10` are per-lane Q32.32 — both bit-identical to the scalar `Fixed` result). **`Float` transcendentals are hand-written in-tree kernels** — there is **no external math library at all**: `exp`, `log`, `log10`, `sin`, `cos`, `tan`, `atan`, `asin`, `acos`, `atan2`, and `pow` are NEON/GPR `f64` kernels, and the `Float MOD Float` operator (`fmod`) is an exact GPR kernel, all identical on every target (macOS / Linux-glibc / Linux-musl). `exp`, `log`, `log10`, `sin`, `cos`, `atan`, `asin`, `acos`, `atan2`, and `pow` are within **≤1 ULP of macOS libm** (double-double-compensated polynomials / fdlibm 4-segment `atan` / `acos` via the `2·atan(√((1−x)/(1+x)))` half-angle identity / fdlibm `__ieee754_pow` in log2 space, including negative base with an integer exponent: `(-2)^3 = -8`). `tan` is **faithfully rounded — ≤1 ULP of the true value** (a double-double sin/cos with a compensated divide); it is in fact more accurate than macOS libm `tan`, which is itself off by >1 ULP at a few inputs. `fmod` is **bit-identical to libm** (the remainder is exact). Every scalar Float overload **shares the same kernel** as its array overload (re-pointed off libm), so `math::f(x)` and `math::f([x])[0]` are bit-identical. The algebraic overloads (`abs`/`min`/`max`/`clamp`/`floor`/`ceil`/`round`/`sqrt`) are exact, matching the scalar result element-wise.
JSON (`IMPORT json`): `json::parse`, `json::stringify`, `json::get`, `json::getOr`.
Error codes (`IMPORT errorCode`): `errorCode::ErrInvalidArgument`, `errorCode::ErrNotFound`, and the other constants listed in the built-in error-code registry. These are compile-time `Integer` constants (`src/builtins/errorcode.rs`), not callables.

> CSV (`IMPORT csv`), HTTP (`IMPORT http`), and datetime (`IMPORT datetime`)
> are additional import-gated packages; see their own documentation.

Fallible built-ins (`fs::openFile`, `toInt`, `collections::get`, …) can fail and auto-propagate like any call.

## 18.3 Overriding general built-ins

The **general (unqualified) built-ins** — `toString`, `len`, `typeName`, the `to*` conversions (`toInt`, `toFloat`, `toFixed`, `toByte`), and the `is*` predicates (`isNumeric`, `isEven`, `isOdd`, `isPositive`, `isNegative`, `isZero`, `isEmpty`, `isNotEmpty`) — are **overridable**: a program or package may declare, e.g., `FUNC toString(value AS Point) AS String` or `FUNC len(value AS Grid) AS Integer`, and a plain `toString(p)` / `len(g)` call binds to that declaration when its argument types match. Resolution is **gap-fill**: the scalar/collection built-in stays authoritative for the types it already supports (a user overload can never shadow `toString(42)`), and an override is consulted only when the built-in rejects the argument types. The override is selected by argument type like any overload. `error` is **not** overridable — it is a reserved primitive that builds the read-only `Error` record (`FUNC error(…)` is a `SYMBOL_RESERVED_BUILTIN_NAME` error).

Implementation: overridability is `src/builtins/general.rs` `is_overridable`
(every general name except `error`); the gap-fill routing is
`resolve_general_builtin_override` in `src/monomorph/lower.rs`, which consults a user
override **only** when `builtins::general::resolve_call` rejects the argument
types, so a user overload can never shadow a type the built-in already handles.
The reserved check is `reserved_builtin_name` (the set is exactly `{ error }`),
enforced in `insert_function` in `src/resolver/mod.rs`.

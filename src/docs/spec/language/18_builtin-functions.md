# 18. Built-in Functions

There are exactly **two scoping tiers** of built-in. This section is about the
**language-level always-in-scope** tier; the import-gated standard packages are
catalogued only for orientation (their full surface lives in their own
documentation, e.g. `mfb man <package>`).

## 18.1 Always-in-scope general built-ins

These **eighteen** names are the *only* callables a program may use with no
`IMPORT` and no package qualifier. The resolver treats exactly this set as
always-in-scope unqualified callables, whitelisting exactly the general built-in set: [[src/resolver/resolution.rs:resolve_callable]] [[src/codegen/builtins/general/mod.rs:is_general_call]]

| Name | Arity | Result | Accepted argument types |
| --- | --- | --- | --- |
| `error(code, message)` | 2 | `Error` | `(Integer, String)` — builds the read-only `Error` record |
| `len(value)` | 1 | `Integer` | `String`, `List OF T`, `Map OF K TO V` |
| `typeName(value)` | 1 | `String` | any `T` (never reads the value) |
| `toString(value[, decimals])` | 1–2 | `String` | `Integer`/`Float`/`Fixed`/`Money`/`Boolean`/`String`/`Byte`/`Scalar`/`List OF Byte`; optional `Byte` precision for `Float`/`Fixed`/`Money` |
| `toInt(value[, base])` | 1–2 | `Integer` | `String`, `Byte`, `Float`, `Fixed`, `Money`, `Scalar`; optional `Integer` `base` (2–36) for `String` radix parsing |
| `toFloat(value)` | 1 | `Float` | `String`, `Integer`, `Fixed`, `Money` |
| `toFixed(value)` | 1 | `Fixed` | `String`, `Integer`, `Float`, `Money` |
| `toByte(value)` | 1 | `Byte` | `Integer`, `Money`, `Scalar` |
| `toMoney(value)` | 1 | `Money` | `String`, `Integer`, `Float`, `Fixed`, `Byte` |
| `toScalar(value)` | 1 | `Scalar` | `Integer` code point, one-scalar `String`, `Byte` |
| `isNumeric(value)` | 1 | `Boolean` | `String` |
| `isEven(value)` / `isOdd(value)` | 1 | `Boolean` | `Integer` |
| `isPositive` / `isNegative` / `isZero` | 1 | `Boolean` | `Integer`, `Float`, `Fixed` |
| `isEmpty(value)` / `isNotEmpty(value)` | 1 | `Boolean` | `String`, `List OF T`, `Map OF K TO V` |

> The conversion name is `toInt` (not `toInteger`). String concatenation `&` is a
> binary **operator**, not a built-in
> function — it is not in this set. [[src/lexer.rs:TokenKind]]
>
> The `is*` predicates are lowered **inline** at a direct call site and **out of
> line** where one is named as a function value, [[src/codegen/builtins/general/mod.rs:builtin_function_id]] so they may be passed as a
> predicate anywhere an ordinary `FUNC` may be. The inlining is an optimization,
> not a restriction on the surface. A value-position reference resolves against
> the type **expected** at that position — a `FUNC(T) AS Boolean` annotation, or
> the element type a higher-order call has already bound — because a bare name
> such as `isPositive` is defined over `Integer`, `Float` and `Fixed` and the
> reference alone does not choose one (bug-368).

All eighteen except `error` are **overridable** (see §18.3). Every other built-in
member named below lives in an **import-gated standard package** and is *not*
in scope without its `IMPORT`. The package set the resolver recognizes is fixed:
`app`, `astrings`, `audio`, `bits`, `canvas`, `collections`, `color`, `crypto`,
`csv`, `datetime`, `encoding`, `errorCode`, `fs`, `http`, `io`, `json`, `math`,
`money`, `net`, `os`, `process`, `regex`, `strings`, `tcp`, `term`, `thread`,
`tls`, `udp`, `vector`. [[src/codegen/builtins/mod.rs:is_builtin_import]] A bare unqualified `find`, `get`, `append`, `print`,
… is a `SYMBOL_UNKNOWN_IDENTIFIER` error; a qualified `io::print` without
`IMPORT io` is a `SYMBOL_UNKNOWN_IMPORT` error. The `app` and `canvas` packages
are additionally gated to `--app` builds: importing either in a plain console
build is a compile-time error (plan-62-A, plan-98-B).

## 18.2 Import-gated standard packages (orientation only)

Each row requires the named `IMPORT`. (`find`/`mid`/`replace` and the
collection accessors share resolver logic but are
reached only through their `strings::`/`collections::` qualifiers — never as
bare names.) [[src/codegen/builtins/general/mod.rs]] The member lists below are **non-exhaustive orientation
snapshots**: each package's authoritative surface is its own call matcher
(and the rendered `mfb man <package>` pages). Where a package's full set is
large, only a representative subset is shown.

Terminal and standard-stream I/O (`IMPORT io`): `io::print`, `io::write`, `io::printError`, `io::writeError`, `io::flush`, `io::isBuffered`, `io::setBuffered`, `io::input`, `io::readLine`, `io::readChar`, `io::readByte`, `io::pollInput`, `io::isInputTerminal`, `io::isOutputTerminal`, `io::isErrorTerminal`. [[src/codegen/builtins/io/mod.rs:register]]
Structured terminal / TUI control (`IMPORT term`): `term::on`, `term::off`, `term::isOn`, `term::setForeground`, `term::setBackground`, `term::setBold`, `term::setUnderline`, `term::showCursor`, `term::hideCursor`, `term::clear`, `term::moveTo`, `term::getForeground`, `term::getBackground`, `term::getBold`, `term::getUnderline`, `term::terminalSize`, `term::didResize`.
Filesystem and file I/O (`IMPORT fs`): `fs::fileExists`, `fs::directoryExists`, `fs::exists`, `fs::readBytes`, `fs::readText`, `fs::writeBytes`, `fs::writeText`, `fs::writeBytesAtomic`, `fs::writeTextAtomic`, `fs::appendBytes`, `fs::appendText`, `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, `fs::createTempFile`, `fs::tempDirectory`, `fs::readLine`, `fs::readAll`, `fs::readAllBytes`, `fs::writeAll`, `fs::writeAllBytes`, `fs::setBuffered`, `fs::isBuffered`, `fs::flush`, `fs::close`, `fs::eof`, `fs::canonicalPath`, `fs::isWithin`, `fs::pathJoin`, `fs::pathDirName`, `fs::pathBaseName`, `fs::pathExtension`, `fs::pathNormalize`, `fs::deleteFile`, `fs::createDirectory`, `fs::createDirectories`, `fs::deleteDirectory`, `fs::listDirectory`, `fs::currentDirectory`, `fs::setCurrentDirectory`.
Process environment and introspection (`IMPORT os`): `os::getEnv`, `os::getEnvOr`, `os::hasEnv`, `os::setEnv`, `os::unsetEnv`, `os::environ`, `os::args`, `os::name`, `os::arch`, `os::pid`, `os::cpuCount`, `os::hostName`, `os::userName`, `os::executablePath`, `os::sleep`. [[src/codegen/builtins/os/mod.rs:register]] `os::getEnv` raises `ErrNotFound` for an unset variable; `os::getEnvOr`/`os::hasEnv` are the non-raising alternatives; `os::environ` returns a `Map OF String TO String` snapshot of the live environment. The introspection calls are nullary and read-only: `os::args` is a `List OF String` of the arguments after the program name; `os::name`/`os::arch` are build-target constants; `os::hostName`/`os::userName`/`os::executablePath` raise `ErrUnsupported` if the host lookup fails.
Network (`IMPORT net`, `IMPORT tcp`, `IMPORT udp`, `IMPORT tls`): `net::lookup`, `net::ping`, `net::toUrl`, `net::percentDecode`, `net::parseQuery`; [[src/codegen/builtins/net/mod.rs:register]] `tcp::connect`, `tcp::listen`, `tcp::accept`, `tcp::read`, `tcp::write`, `tcp::poll`, `tcp::close`, `tcp::localAddress`, `tcp::remoteAddress`, `tcp::setReadTimeout`, `tcp::setWriteTimeout`; [[src/codegen/builtins/tcp/mod.rs:register]] `udp::bind`, `udp::send`, `udp::receive`, `udp::poll`, `udp::close`, `udp::localAddress`, `udp::setReadTimeout`, `udp::setWriteTimeout`; [[src/codegen/builtins/udp/mod.rs:register]] `tls::connect`, `tls::listen`, `tls::accept`, `tls::read`, `tls::write`, `tls::poll`, `tls::close`, `tls::localAddress`, `tls::remoteAddress`, `tls::setReadTimeout`, `tls::setWriteTimeout` (the additional `tls::closeListener` member is an internal listener-close dispatch target, not a user-callable surface — `tls::close` over a `tls::Listener` rewrites to it during IR lowering). [[src/codegen/builtins/tls/mod.rs:register]] plan-110 split the former single `net` transport surface across these four packages: `read` returns bytes everywhere and `write` takes a `String` as an overload, so the `readText`/`writeText` members are gone. There is deliberately **no** `tls::wrap`: upgrading an established `tcp::Socket` in place would need to adopt its descriptor, and macOS exposes no supported API that can (`nw_connection_create_with_connected_socket` is undeclared SPI that rejects an adopted fd, and Secure Transport is deprecated and cannot negotiate TLS 1.3). Shipping it on Linux and Windows alone would make a program compile for every target and fail at runtime on one, so the member does not exist anywhere (plan-110-D §C9).
Strings (`IMPORT strings`, representative subset — full set in the strings call matcher [[src/codegen/registry/mod.rs:is_member]]): `strings::find`, `strings::mid`, `strings::replace`, `strings::trim`, `strings::trimStart`, `strings::trimEnd`, `strings::trimChars`, `strings::upper`, `strings::lower`, `strings::caseFold`, `strings::normalizeNfc`, `strings::graphemes`, `strings::graphemeAt`, `strings::graphemesCount`, `strings::startsWith`, `strings::endsWith`, `strings::startsWithAny`, `strings::endsWithAny`, `strings::stripPrefix`, `strings::stripSuffix`, `strings::contains`, `strings::count`, `strings::left`, `strings::right`, `strings::repeat`, `strings::padLeft`, `strings::padRight`, `strings::split`, `strings::join`, `strings::byteLen`. (`len`, `toString`, `toInt`, `toFloat`, `toFixed`, `toByte`, `isNumeric` are general always-in-scope built-ins, §18.1; `&` is the concatenation operator.)
Regex (`IMPORT regex`): `regex::match`, `regex::find`, `regex::findAll`, `regex::replace`.
Collections (`IMPORT collections`): the migrated native accessors `collections::forEach`, `collections::transform`, `collections::filter`, `collections::reduce`, `collections::sum`, `collections::get`, `collections::getOr`, `collections::find`, `collections::mid`, `collections::replace`, `collections::set`, `collections::append`, `collections::prepend`, `collections::insert`, `collections::removeAt`, `collections::removeKey`, `collections::keys`, `collections::values`, `collections::hasKey`, `collections::contains`, [[src/codegen/builtins/collections/mod.rs:register]] plus the MFBASIC-source generics `collections::sort`, `sortBy`, `take`, `drop`, `reduceRight`, `any`, `all`, `findIndex`, `findLastIndex`, `groupBy`, `mapValues`, `flatten`, `zip`, `chunks`, `window`, `distinct`, `merge`, `partition`. (`len` of a `List`/`Map` is the general built-in, §18.1.)
Threads (`IMPORT thread`) [[src/codegen/builtins/thread/mod.rs:is_thread_call]]: `thread::start`, `thread::isRunning`, `thread::waitFor`, `thread::cancel`, `thread::send`, `thread::poll`, `thread::receive`, `thread::isCancelled`, and the resource/value transfer-plane members `thread::transfer`, `thread::accept`, `thread::transferResource`, `thread::acceptResource`, `thread::emitResource`, `thread::readResource`.
Math (`IMPORT math`): the call members `math::abs`, `math::min`, `math::max`, `math::clamp`, `math::floor`, `math::ceil`, `math::round`, `math::sqrt`, `math::pow`, `math::exp`, `math::log`, `math::log10`, `math::sin`, `math::cos`, `math::tan`, `math::asin`, `math::acos`, `math::atan`, `math::atan2`, `math::rand`, `math::seed`, [[src/codegen/builtins/math/mod.rs:is_math_call]] and the compile-time constants `math::pi`, `math::piFixed`, `math::e`, `math::eFixed` (which fold to literals like the `errorCode::Err*` registry — not callables). [[src/codegen/builtins/math/mod.rs:is_math_constant]]

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
Error codes (`IMPORT errorCode`): `errorCode::ErrInvalidArgument`, `errorCode::ErrNotFound`, and the other constants listed in the built-in error-code registry. These are compile-time `Integer` constants, not callables. [[src/codegen/builtins/errorcode/mod.rs]]

> CSV (`IMPORT csv`), HTTP (`IMPORT http`), and datetime (`IMPORT datetime`)
> are additional import-gated packages; see their own documentation.

Fallible built-ins (`fs::openFile`, `toInt`, `collections::get`, …) can fail and auto-propagate like any call.

## 18.3 Overriding general built-ins

The **general (unqualified) built-ins** — `toString`, `len`, `typeName`, the `to*` conversions (`toInt`, `toFloat`, `toFixed`, `toByte`), and the `is*` predicates (`isNumeric`, `isEven`, `isOdd`, `isPositive`, `isNegative`, `isZero`, `isEmpty`, `isNotEmpty`) — are **overridable**: a program or package may declare, e.g., `FUNC toString(value AS Point) AS String` or `FUNC len(value AS Grid) AS Integer`, and a plain `toString(p)` / `len(g)` call binds to that declaration when its argument types match. Resolution is **gap-fill**: the scalar/collection built-in stays authoritative for the types it already supports (a user overload can never shadow `toString(42)`), and an override is consulted only when the built-in rejects the argument types. The override is selected by argument type like any overload. `error` is **not** overridable — it is a reserved primitive that builds the read-only `Error` record (`FUNC error(…)` is a `SYMBOL_RESERVED_BUILTIN_NAME` error).

Implementation: every general name except `error` is overridable; [[src/codegen/builtins/general/mod.rs:is_overridable]] the gap-fill routing consults a user
override **only** when the built-in's own resolution rejects the argument
types, so a user overload can never shadow a type the built-in already handles. [[src/monomorph/lower.rs:resolve_general_builtin_override]]
The reserved check covers exactly the set `{ error }`,
enforced when the resolver inserts a function. [[src/codegen/builtins/general/mod.rs:reserved_builtin_name]] [[src/resolver/mod.rs:insert_function]]

## 18.4 Timeout convention

Every built-in that can **wait** takes an optional trailing `timeoutMs AS Integer`
and interprets it **identically**. This is the one normative rule; a family's only
freedom is whether a given function is a *readiness query* or a *producing call*
(defined below). There is no per-package variation in the meaning of the value.

| `timeoutMs` | Meaning | Readiness query returns | Producing call does |
|---|---|---|---|
| omitted | unbounded — block until the event or a terminal condition (closed / cancelled / EOF / OS refusal) | (n/a — waits) | (n/a — waits) |
| `0` | one immediate, non-blocking attempt | current-state value (`FALSE` / `-1`) | the event if already available, else `ErrTimeout` |
| `> 0` | wait up to that many ms, clamped to `2147483647` where the host takes a C `int` | not-ready value on the deadline | `ErrTimeout` on the deadline |
| `< 0` | rejected | `ErrInvalidArgument` (77050002) | `ErrInvalidArgument` (77050002) |

- A **readiness query** has a not-ready value to return (`FALSE` for a
  `Boolean` poll, `-1`, etc.): the scalar `Socket → Boolean` form of `tcp::poll`,
  `udp::poll` and `tls::poll`; `audio::poll`; `io::pollInput`.
- A **producing call** yields a resource, message, connection, or bytes and has
  no not-ready value, so an unmet deadline is an error: `tcp::accept`,
  `tcp::connect`, the multiplex `List OF RES <pkg>::Socket → <pkg>::Socket` form of
  `tcp::poll`/`udp::poll`/`tls::poll` (which yields the first ready socket),
  `tcp::read`/`write`, `udp::receive`/`send`, `tls::read`/`write`
  (all under a socket read/write timeout), `tls::connect`, `tls::accept`, `audio::read`,
  `thread::send`, `thread::receive`, `thread::transfer`, `thread::accept`.
- **Expiry raises exactly one error, `ErrTimeout` (77050008)**, for every
  producing call. There is no family-specific expiry code. (The socket read/write
  members formerly raised `ErrReadTimeout`/`ErrWriteTimeout`; thread `receive`/`accept`
  at `0` formerly raised `ErrNotFound`. Both were retired for this convention.)
- The **socket-option setters** `setReadTimeout`/`setWriteTimeout` (on `tcp`,
  `udp` and `tls` alike) bind
  a socket's subsequent read/write deadline rather than waiting themselves: `0`
  makes those operations non-blocking (immediate `ErrTimeout` when not ready),
  `> 0` bounds them, `< 0` is `ErrInvalidArgument`. A fresh socket is unbounded;
  the setter can only bound, so unbounded is not restorable through it.

Omitting the argument is implemented by padding it with the internal
"wait unbounded" sentinel (the `i64::MIN` bit pattern); each family's wait helper
routes that sentinel to its block-forever path and rejects every other negative
value. [[src/codegen/error/constants/error_constants.rs:TIMEOUT_UNBOUNDED_SENTINEL]]

**Conforming functions** (every waiting built-in obeys the table above):
`tcp::connect`, `tcp::accept`, `tcp::poll`, `tcp::read`, `tcp::write`,
`tcp::setReadTimeout`, `tcp::setWriteTimeout`, `udp::poll`, `udp::receive`,
`udp::send`, `udp::setReadTimeout`, `udp::setWriteTimeout`, `tls::connect`,
`tls::accept`, `tls::poll`, `tls::read`, `tls::write`, `tls::setReadTimeout`,
`tls::setWriteTimeout`, `audio::poll`, `audio::read`, `io::pollInput`,
`thread::send`, `thread::receive`, `thread::transfer`, `thread::accept`.
Conformance was completed across the codebase in one pass; `thread` was the pilot.

(`thread::poll` requires its `ms` argument and has no omit form; it already
rejects negatives and treats `0` as an immediate check, so its value meanings
match the table — it simply does not offer the unbounded/omit spelling.)

## See Also

* ./mfb spec language types — the numeric conversions (`toInt`/`toFloat`/`toFixed`/…) these built-ins perform
* ./mfb spec language modules-and-packages — the `IMPORT` gating for the standard packages catalogued here
* ./mfb man collections — a representative import-gated package surface
* ./mfb man strings — another representative import-gated package surface

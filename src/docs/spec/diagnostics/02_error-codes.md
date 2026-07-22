# Runtime Error Codes

The `errorCode::` package is a flat set of `Integer` constants, one per row of the
runtime error registry. A reference such as `errorCode::ErrNotFound` types as
`Integer` and folds to an integer literal before lowering — there is no runtime
helper, no codegen, and no binary-representation change, mirroring the `math::pi`
constant mechanism. Constants are keyed package-qualified (`"errorCode.<Name>"`)
and resolved by exact match against the generated table. [[src/builtins/errorcode.rs:constant_value]]

These integers are exactly the values runtime code stamps into `Error.code` when
a fallible operation fails (see *See Also*) — both the native codegen/runtime
helpers [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] and the
embedded MFBASIC standard packages (`regex`, `datetime`, `csv`, `json`, `http`,
`net`, …) fail with registry values, and user code may `FAIL` with them too. They
are program-visible data, not host-tool diagnostics; the compiler-facing rule
set (see `./mfb spec diagnostics rule-codes`) is a separate registry and is not
surfaced here.

## Encoding Rule

The canonical code string has the hyphenated form `G-SSS-EEEE`:

* `G` — generator (`7` for runtime codes, `9` for user-defined codes)
* `SSS` — subsystem
* `EEEE` — concrete error within that subsystem

The runtime `Error.code` integer is the canonical code with the hyphens removed.
For example `7-705-0002` is stored as `77050002`. The mapping is exactly
`code.replace('-', "")`; the build step asserts that hyphen-stripping the code
column equals the integer column for every row, so the two cannot drift. [[src/builtins/errorcode.rs:ERRORCODE_CONSTANTS]]

## Subsystem Partitioning

All runtime codes use generator `7`. The subsystems actually present in the
registry are:

| Prefix    | Subsystem                                                        |
|-----------|------------------------------------------------------------------|
| `7-701-*` | memory and allocation                                            |
| `7-702-*` | I/O                                                              |
| `7-703-*` | filesystem and resource handles                                  |
| `7-705-*` | package helpers, builtins, and generic language errors           |
| `7-706-*` | trap and failure propagation                                     |
| `7-707-*` | platform ABI and networking                                      |

(`7-704-*` is unused by the current registry.)

## User-Defined Codes (generator `9`)

Generator `9` — the integer range `90000000` through `99999999` — is reserved for
codes a package or program defines for itself. **The runtime never raises a
generator-`9` code, and no such code appears in the `errorCode::` registry.** It
exists so a library with failure modes the standard registry does not model can
report them without overlaying an unrelated standard meaning.

The problem it solves is concrete. A binding wrapping a C library has its own
error enum, and mapping that enum onto the `7-705-*` range by arithmetic offset
lands on unrelated standard names: a "format not recognised" reported as
`77050001` is indistinguishable from `errorCode::ErrIndexOutOfRange`, so a
`TRAP` matching that constant silently catches the wrong failure. Generator `9`
gives that binding a range where it cannot collide with a standard name.

Rules:

* **Not in `errorCode::`.** There is no constant for a generator-`9` code, and
  `errorCode::` will never gain one. A handler compares against the package's own
  documented integer, or against a constant the package exports itself.
* **Codes may overlap between packages.** `SSS` is chosen by whoever defines the
  code, with no central registry and no allocation authority, so two unrelated
  packages may both use `9-100-0001` for different failures. A code is meaningful
  only together with the package that raised it — never treat a generator-`9`
  integer as globally identifying.
* **Because they overlap, match them narrowly.** A `TRAP` keying on a
  generator-`9` code should already know which package it is handling — typically
  by wrapping a single call. Matching one across a broad block risks catching a
  same-numbered code from an unrelated package.
* **Document every one.** A generator-`9` code carries no registry meaning, so
  the raising package's `DOC` `ERROR` lines are the only specification of what it
  means. An undocumented one is unusable by a caller.
* **Prefer a standard code where one genuinely fits.** Generator `9` is for
  failures the registry does not model, not a default. A missing file really is
  `ErrPathNotFound`; a bad argument really is `ErrInvalidArgument`. Reach for
  generator `9` only when no standard code carries the right meaning.

The encoding rule is unchanged: `9-100-0001` is stored as `91000001`, and the
value is built with the ordinary `error(code, message)` built-in. Nothing in the
compiler or runtime treats the range specially — the reservation is a convention
that keeps user codes from being mistaken for registry codes.

## Constant Registry

The complete `errorCode::` Name → Integer mapping. This table is the build input
from which `ERRORCODE_CONSTANTS` is generated (see *Drift Guard*); row order is
registry order. [[src/builtins/errorcode.rs:ERRORCODE_CONSTANTS]]

| Code         | Integer    | Name                          | Meaning |
|--------------|------------|-------------------------------|---------|
| `7-705-0000` | `77050000` | `ErrUnknown`                  | Unclassified standard-package failure. |
| `7-705-0001` | `77050001` | `ErrIndexOutOfRange`          | List or string index/range is outside valid bounds. |
| `7-705-0002` | `77050002` | `ErrInvalidArgument`          | Argument value is not valid for the requested operation. |
| `7-705-0003` | `77050003` | `ErrInvalidFormat`            | Text parse or non-finite numeric representation conversion failed. |
| `7-705-0004` | `77050004` | `ErrNotFound`                 | Requested item, key, file, or resource was not found. |
| `7-705-0005` | `77050005` | `ErrAlreadyExists`            | Create operation conflicts with an existing item. |
| `7-705-0006` | `77050006` | `ErrPermissionDenied`         | Operation is not permitted by the host environment. |
| `7-705-0007` | `77050007` | `ErrUnsupported`              | Operation is not supported by the implementation or platform. |
| `7-705-0008` | `77050008` | `ErrTimeout`                  | Operation did not complete before its deadline. |
| `7-705-0009` | `77050009` | `ErrInterrupted`              | Operation was interrupted before completion. |
| `7-701-0001` | `77010001` | `ErrOutOfMemory`              | Allocation failed. |
| `7-703-0001` | `77030001` | `ErrPathNotFound`             | Filesystem path does not exist. |
| `7-703-0002` | `77030002` | `ErrInvalidPath`              | Filesystem path string is invalid for the host platform. |
| `7-703-0003` | `77030003` | `ErrAccessDenied`             | Filesystem access was denied. |
| `7-702-0001` | `77020001` | `ErrReadFailed`               | Read operation failed. |
| `7-702-0002` | `77020002` | `ErrWriteFailed`              | Write or flush operation failed. |
| `7-702-0003` | `77020003` | `ErrEndOfFile`                | Read operation reached end of file where a value was required. |
| `7-703-0004` | `77030004` | `ErrResourceClosed`           | Resource handle is already closed. |
| `7-703-0005` | `77030005` | `ErrResourceBusy`             | Resource is unavailable, locked, busy, or not in the required empty state. |
| `7-702-0004` | `77020004` | `ErrEncoding`                 | Text encoding or decoding failed. |
| `7-702-0005` | `77020005` | `ErrInputFailed`              | Standard input operation failed. |
| `7-707-0001` | `77070001` | `ErrAddressInvalid`           | Network host, address, or port is invalid. |
| `7-707-0002` | `77070002` | `ErrAddressNotFound`          | Network host name or address could not be resolved. |
| `7-707-0003` | `77070003` | `ErrNetworkFailed`            | Network operation failed before a connection was established. |
| `7-707-0004` | `77070004` | `ErrConnectionClosed`         | Socket peer closed the connection or the connection is no longer usable. |
| `7-707-0005` | `77070005` | `ErrReadTimeout`              | Socket read operation timed out. |
| `7-707-0006` | `77070006` | `ErrWriteTimeout`             | Socket write operation timed out. |
| `7-707-0007` | `77070007` | `ErrMessageTooLarge`          | Datagram or message exceeds the requested or supported size. |
| `7-705-0010` | `77050010` | `ErrOverflow`                 | Arithmetic overflow or numeric conversion outside the destination range. |
| `7-703-0006` | `77030006` | `ErrCloseFailed`              | Resource close operation failed. |
| `7-703-0007` | `77030007` | `ErrNativeBindingUnavailable` | Native `LINK` binding library or symbol could not be loaded at startup (`dlopen`/`dlsym` failed). |
| `7-703-0008` | `77030008` | `ErrNativeBindingCallFailed`  | Native `LINK` binding call failed its `SUCCESS_ON` gate. |
| `7-707-0008` | `77070008` | `ErrTlsFailed`                | TLS handshake, certificate validation, SNI validation, or protocol operation failed. |
| `7-705-0011` | `77050011` | `ErrUnderflow`                | Arithmetic underflow below the destination range. |
| `7-705-0012` | `77050012` | `ErrFloatDomain`              | Floating-point operation domain is invalid (negative `sqrt`, non-positive `log`/`log10`, out-of-range `asin`/`acos`, a non-whole or negative `^` exponent, or a `Float MOD 0`). Divide-by-zero is not reported here — `x / 0` produces `±Inf`/`NaN` caught at the observation boundary as `ErrFloatOverflow`/`ErrFloatNaN`. |
| `7-705-0013` | `77050013` | `ErrFloatNaN`                 | Floating-point operation produced a NaN result. |
| `7-705-0014` | `77050014` | `ErrFloatInf`                 | Floating-point operation produced an infinity result. |
| `7-705-0015` | `77050015` | `ErrFloatOverflow`            | Floating-point arithmetic overflowed to infinity. |
| `7-706-0001` | `77060001` | `ErrWrapped`                  | Generic wrapper code for adding context while preserving the underlying message. |
| `7-705-0016` | `77050016` | `ErrAuthenticationFailed`     | Authenticated decryption failed: the message authentication tag did not verify. |
| `7-705-0017` | `77050017` | `ErrAudioUnavailable`         | Audio backend library or device is unavailable (no `libasound.so.2`, no audio device, or capture authorization denied). |
| `7-705-0018` | `77050018` | `ErrAudioDevice`              | Audio device open, configuration, or stream operation failed. |
| `7-705-0019` | `77050019` | `ErrInvalidContext`          | Operation was invoked from a thread that is not permitted to perform it (e.g. reading stdin from a thread that has not called `thread::openStdIn`). |
| `7-703-0009` | `77030009` | `ErrResourceMoved`            | Resource handle was moved to another thread by `thread::transfer` and is no longer usable by the sender. |

## Resolution API

Constant resolution answers three questions about a package-qualified name —
whether it is a known constant, that its type is `Integer`, and the folded
integer literal it becomes. Resolution strips the `errorCode.` prefix and
rejects unqualified or unknown names. [[src/builtins/errorcode.rs:constant_value]]

## Drift Guard

The `errorCode` constants are generated at build time directly from the
**Constant Registry** table above — this topic is the single source of truth
for the runtime registry. [[build.rs:generate_errorcode_table]] A drift-guard
test re-parses the same table and asserts the generated table reproduces every
row with the integer equal to the hyphen-stripped code, so the generated
constants cannot drift from this registry. [[src/builtins/errorcode.rs:table_matches_registry]]

## See Also

* ./mfb spec memory fallible-call-abi — the `Error` value and the result-value register these integers populate
* ./mfb spec language error-model — the source-level error/TRAP/FAIL model that produces `Error.code`
* ./mfb spec language types — `Integer` and `Error` types

# Runtime Error Codes

The `errorCode::` package is a flat set of `Integer` constants, one per row of the
runtime error registry. A reference such as `errorCode::ErrNotFound` types as `Integer`
and folds to an integer literal through the shared package-constant lowering path;
reading a constant requires no runtime lookup or helper call. Constants are keyed
package-qualified (`"errorCode.<Name>"`) and resolved by exact match against the
registered constants. [[src/codegen/registry/mod.rs:constant_value]]
[[src/ir/lower.rs:lower_expression_with_expected]]

These integers are exactly the values runtime code stamps into `Error.code` when a
fallible operation fails (see *See Also*) — both the native codegen/runtime helpers
[[src/codegen/builtins/errorcode/mod.rs:register]] and the embedded MFBASIC standard
packages (`regex`, `datetime`, `csv`, `json`, `http`, `net`, …) fail with registry
values, and user code may `FAIL` with them too. They are program-visible data, not
host-tool diagnostics; the compiler-facing rule set (see `./mfb spec diagnostics
rule-codes`) is a separate registry and is not surfaced here.

## Encoding Rule

The canonical code string has the hyphenated form `G-SSS-EEEE`:

* `G` — generator (`7` for runtime codes, `9` for user-defined codes)
* `SSS` — subsystem
* `EEEE` — concrete error within that subsystem

The runtime `Error.code` integer is the canonical code with the hyphens removed. For
example `7-705-0002` is stored as `77050002`. The mapping is exactly `code.replace('-',
"")`; the `table_matches_registry` test asserts that hyphen-stripping the code column
equals the integer column for every row, as well as comparing the registered constants.
[[src/codegen/builtins/errorcode/mod.rs:table_matches_registry]]

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
| `7-708-*` | child processes                                                  |

(`7-704-*` is unused by the current registry.)

## User-Defined Codes (generator `9`)

Generator `9` — integer range `90000000` through `99999999` — is the convention for
package- or program-defined errors. The standard `errorCode::` registry contains no
generator-`9` constants. There is no central allocation of user subsystems; codes from
different packages can overlap and do not identify a failure independently of its
originating package. [[src/codegen/builtins/errorcode/mod.rs:register]]

The same encoding convention maps `9-100-0001` to `91000001`. The ordinary `error(code,
message)` builtin accepts an `Integer`; it does not enforce this range or reserve it at
runtime. User-defined values propagate through the same `Error` representation as
standard codes. See `./mfb spec language error-model`.
[[src/codegen/builtins/general/func_error.rs:register]]

## Constant Registry

The complete `errorCode::` Name → Integer mapping. This table specifies the names and
values registered by `errorcode::register` (see *Drift Guard*); row order is
registration order. Meanings describe error categories, not an exhaustive list of
emitting operations or a guarantee of identical message text at every emitter.
[[src/codegen/builtins/errorcode/mod.rs:register]]

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
| `7-701-0002` | `77010002` | `ErrStackOverflow`            | The call depth exceeded the thread's stack. Raised at entry to the function that would have run past the thread's stack limit, before it binds anything; the caller's `TRAP` catches it and every frame unwound runs its cleanup, as for any failure. Distinct from `ErrOutOfMemory`: the arena may have room — the call chain is too deep. |
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
| `7-705-0020` | `77050020` | `ErrWrongMode`               | Operation requires a presentation mode the program is not in. In an `--app` build, `term::*` requires `app::Mode.Console` — the character grid exists only there, so `None` and `Canvas` both trap. The console-reading `io::` calls (`io::input`, `io::readLine` and the keystroke readers `io::readKey`/`readScalar`/`readGrapheme`/`readRaw`) require only a window to take key events from, so they trap in `Mode.None` alone and work in both `Console` and `Canvas`. |
| `7-703-0009` | `77030009` | `ErrResourceMoved`            | Resource handle was moved to another thread by `thread::transfer` and is no longer usable by the sender. |
| `7-703-0010` | `77030010` | `ErrNativeBufferOverrun`      | Native `LINK` `OUT CBuffer` callee wrote past its declared `SIZE` (buffer overrun detected). |
| `7-708-0001` | `77080001` | `ErrSpawnFailed`              | Child process could not be spawned (fork/exec failed, or the program was not found). |
| `7-705-0021` | `77050021` | `ErrBadPixelCount`            | Pixel list length does not match the image dimensions: an RGBA8 image needs exactly `width * height * 4` bytes (`canvas::createImage`, `canvas::setBytes`). |
| `7-705-0022` | `77050022` | `ErrBadFontFile`             | File is not a font this build can read: `canvas::loadFont` takes TrueType outlines (sfnt version `0x00010000` or `true`, alone or as a face of a font collection) and refuses CFF/OpenType-PostScript and WOFF, an empty collection or a selected face without TrueType outlines, and a TrueType file whose `head.unitsPerEm` is outside 16..16384. Distinct from `ErrPathNotFound` for a missing path and `ErrNotFound` for an unmatched font face name. [[src/codegen/builtins/canvas/func_load_font.rs:LOAD_FONT]] [[src/codegen/builtins/canvas/func_load_font.rs:LOAD_FONT_NAMED]] |
| `7-705-0023` | `77050023` | `ErrBadImageFile`            | File is not an image this build can decode: `canvas::loadImage` reads PNG (all five colour types, bit depths 1–16, interlaced or not) and refuses anything else, including a PNG whose chunks, filters or compressed data are malformed, that is larger than 16384 pixels a side or 16,777,216 pixels, or whose compressed data does not fit the image it declares. Distinct from `ErrPathNotFound`, which reports a missing path. [[src/codegen/builtins/canvas/func_load_image.rs:LOAD_IMAGE]] [[src/codegen/builtins/fs/func_read_bytes.rs:register]] |
| `7-705-0024` | `77050024` | `ErrDepthExceeded`           | Structural nesting exceeds the implementation depth limit. Distinct from `ErrInvalidFormat`: processing stops at the depth limit without validating the remaining structure (`json::parse` stops at 256 nested arrays and objects; `canvas::present` stops at 64 nested `canvas::Group` references, which is also how a group that eventually references itself is reported — a cycle is unbounded depth). |
| `7-705-0025` | `77050025` | `ErrInvalidSurrogate`        | A `\u` escape encodes an unpaired surrogate. Strings are Unicode text, so a high surrogate must be followed by a `\u` low surrogate, and a lone low surrogate is never valid. Distinct from `ErrInvalidFormat`, which is a grammar mistake. |
| `7-705-0026` | `77050026` | `ErrCanvasGroupLimit`        | The canvas named-group table is full: `canvas::setGroup` holds at most 256 groups at once. Remove one with `canvas::removeGroup`, or install fewer — installing again under a name you already used replaces it and needs no new slot. Distinct from `ErrOutOfMemory`: the arena has room, the fixed table does not.  |
| `7-705-0027` | `77050027` | `ErrTooLarge`                | The result would be larger than the size limit the caller set. Distinct from `ErrOutOfMemory`: the input can be perfectly valid and memory can be available — raise the limit (for example `maxBytes` on `compress::inflate`) when the size is expected, or reject the input. |
| `7-705-0028` | `77050028` | `ErrCanvasParticleLifetime`  | A `canvas::ParticleSystem` has a `lifetime` of zero or less. A particle's colour and size are worked out from how far it is through its life, and there is no such point in a life with no length — give `emission.lifetime` a value above zero. A system that should draw nothing wants `maxParticles` of zero instead. |

## Resolution API

Constant resolution answers three questions about a package-qualified name — whether it
is a known constant, its declared type (`Integer` for every `errorCode` member), and the
folded integer literal it becomes. Resolution splits the qualified name at the first
dot, resolves the package, and matches the constant member exactly. Unqualified or
unknown names have no constant value. [[src/codegen/registry/mod.rs:find_constant]]
[[src/codegen/registry/mod.rs:constant_value]]

## Drift Guard

The `errorCode` constants are a hand-maintained table in the package's `register`, which
must reproduce the **Constant Registry** table above — this topic is the single source
of truth for the runtime registry. [[src/codegen/builtins/errorcode/mod.rs:register]] A
drift-guard test re-parses the same table and asserts the registered table reproduces
every row with the integer equal to the hyphen-stripped code, including registration
order. It does not compare the prose meanings or runtime message text. This is a
test-time check; `build.rs` embeds the spec pages but does not generate these constants.
[[build.rs:main]] [[src/codegen/builtins/errorcode/mod.rs:table_matches_registry]]

## See Also

* ./mfb spec memory fallible-call-abi — the `Error` value and the result-value register these integers populate
* ./mfb spec language error-model — the source-level error/TRAP/FAIL model that produces `Error.code`
* ./mfb spec language types — `Integer` and `Error` types

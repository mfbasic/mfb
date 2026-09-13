# link

Native LINK declarations for binding host dynamic libraries

## Synopsis

```
mfb man link
```

## Imports

`link` is a developer documentation topic, not an importable package. `LINK`,
`RESOURCE`, `SYMBOL`, `ABI`, `CONST`, `SUCCESS_ON`, `ERROR_ON`, `RETURN`, `FREE`,
`CSTRUCT`, `BIND IN`, `BIND STATE`, and `BUFFER` are source forms used inside a
binding package (the last four under *Structs and buffers* below).

## Description

A `LINK` block declares the native surface of a reusable binding package. It
names one host dynamic library, gives that library a package-local alias, and
declares typed MFBASIC wrapper functions for native symbols in that library.
Application packages do not repeat the `LINK` block. They import the compiled
binding package, call its exported wrappers, and use any exported resource types like any other `RES` handle (see
`mfb man variable`).

```
EXPORT RESOURCE Db CLOSE BY sqlite::close

LINK "sqlite3" AS sqlite
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, db OUT CPtr) AS status CInt32
    RETURN db
    SUCCESS_ON status = 0
  END FUNC

  FUNC close(RES db AS Db) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

EXPORT FUNC close AS sqlite::close
```

`LINK "sqlite3" AS sqlite` creates the package-local namespace `sqlite`.
Members are referenced as `sqlite::open` and `sqlite::close` inside the binding
package. `LINK` aliases are
visible before other top-level names, so a resource's `CLOSE BY` and a transparent
re-export alias can refer to a native function declared later in the file. A `LINK` alias is distinct from an imported
package and wins before import lookup for that root name.

## Binding packages

A source package that declares `LINK` is a binding package. It may contain only
the native declarations, or it may add ordinary MFBASIC wrapper code around them
for validation, safer defaults, and higher-level APIs. The compiled `.mfp`
contains normal package metadata plus native binding metadata, so importers see
ordinary package functions and resource types.

Native resources are declared at package scope with
`RESOURCE Name CLOSE BY alias::func`, not inside the `LINK` block. The close
function must be a native `LINK` function that takes exactly one `RES`
parameter of that resource type and closes it. A transparent function alias,
`EXPORT FUNC close AS alias::func`, is the way to expose that same close
operation to importers.

## Native functions

Each native `FUNC` has two signatures:

- The MFBASIC-facing signature after `FUNC`, using source types such as
  `String`, `Integer`, `Float`, `Nothing`, and `RES Db`.
- The C-facing ABI signature after `ABI`, using ABI slot types such as
  `CString`, `CPtr`, `CInt32`, `CBool`, `CByte`, `CDouble`, and `CVoid`.

`SYMBOL "name"` gives the exact dynamic-library symbol to resolve. `ABI (...)`
lists native arguments in C call order, and `AS slot CType` names the native
return slot. ABI slots bind to wrapper parameters by name. Every wrapper
parameter must have a matching ABI slot, and every input ABI slot must be a
wrapper parameter or a `CONST` pin. An `OUT` slot is storage the native function
fills, so it needs neither.

A value-returning wrapper names its result with exactly one
`RETURN <expression>`. The expression may name an `OUT` slot (`RETURN db`) or
the native return, or compute a value from the slots
(`RETURN status = 100` for a `Boolean` result). A `Nothing` wrapper has no
`RETURN`. A function may declare several `OUT` slots, but only the one `RETURN`
names reaches the program. Slot names are ordinary identifiers; `return` is a
keyword and cannot name a slot.

## Result gates

`SUCCESS_ON` and `ERROR_ON` describe when a native call succeeds. The condition
is a Boolean expression over ABI slot names:

```
SUCCESS_ON status = 0
ERROR_ON status = -1
SUCCESS_ON status = 100 OR status = 101
```

When the gate says the call failed, the wrapper fails with
`ErrNativeBindingCallFailed` and ordinary MFBASIC error propagation applies.
`CONST` pins provide fixed ABI slot values without exposing them as wrapper
parameters. `FREE <slot>` runs a declared native deallocator on a produced `CPtr`
slot once its value has been copied into an MFBASIC value.

## Structs and buffers

A C function that reads or fills a struct needs the struct's exact layout.
`CSTRUCT <CName> AS <Record>` declares it inside the `LINK` block — fields one per
line, in C declaration order — together with the ordinary record the program
sees instead. The compiler works out offsets and padding; the struct and the
record must have the same field names with compatible types
(`NATIVE_STRUCT_FIELD_MISMATCH`). A `CSTRUCT` name is usable only inside its
`LINK` block, in an `ABI` slot or after `SIZEOF`; everywhere else the program
names the record.

```
TYPE AudioFormat
  format    AS Integer
  name      AS String
  extension AS String
END TYPE

LINK "sndfile" AS snd
  CSTRUCT SfFormatInfo AS AudioFormat
    format     CInt32
    name       CString
    extension  CString
  END CSTRUCT

  FUNC getFormat(index AS Integer) AS AudioFormat
    SYMBOL "sf_command"
    ABI (handle CPtr, command CInt32, info INOUT SfFormatInfo, datasize CInt32) AS status CInt32
    CONST handle = NOTHING
    CONST command = 4129
    CONST datasize = SIZEOF SfFormatInfo
    BIND IN info
      format = index
    END BIND
    RETURN info
    SUCCESS_ON status = 0
  END FUNC
END LINK
```

A struct slot takes a direction: `IN` (the default) when the native function only
reads it, `OUT` when it only fills it, `INOUT` for both. Every field starts at
zero. `BIND IN <slot> … END BIND` sets fields before the call from wrapper
parameters or integer literals, so `getFormat(3)` needs no whole record from the
caller. `RETURN <slot>` hands the filled struct back as its record.
`SIZEOF <CName>` is the struct's size in bytes, for C APIs that ask for it.

`BIND STATE <result-slot> = <struct-slot>` fills the `STATE` of a returned
resource from an `OUT` struct slot, where `<result-slot>` is the slot `RETURN`
names. With `SfFileInfo` declared as a `CSTRUCT` of the record `FileInfo`:

```
  FUNC openFile(path AS String) AS RES SoundFile STATE FileInfo
    SYMBOL "sf_open"
    ABI (path CString, mode CInt32, info OUT SfFileInfo) AS file CPtr
    CONST mode = 16
    BIND STATE file = info
    ERROR_ON file = NOTHING
    RETURN file
  END FUNC
```

The program then reads the filled record as `.state` on the handle.

For a bulk read, a `CBuffer` slot is a byte list the native function fills. It
must be `OUT`, carry exactly one `BUFFER <slot> SIZE <bytes>` clause, and be the
slot `RETURN <slot> LENGTH <bytes>` names; the wrapper returns `List OF Byte`
(`NATIVE_BUFFER_INVALID` otherwise). `SIZE` is worked out before the call, from
the wrapper's parameters and `CONST` pins only, and must be at least the most the
function can write. `LENGTH` is worked out after the call, from the native return
and `OUT` slots, and becomes the list's length; below zero counts as zero, and
above `SIZE` counts as `SIZE`.

```
  FUNC readFrames(RES file AS SoundFile, frames AS Integer, channels AS Integer) AS List OF Byte
    SYMBOL "sf_readf_short"
    ABI (file CPtr, buf OUT CBuffer, frames CInt64) AS read CInt64
    BUFFER buf SIZE frames * channels * 2
    RETURN buf LENGTH read * channels * 2
  END FUNC
```

A native function that writes past `SIZE` fails the call with
`ErrNativeBufferOverrun` instead of continuing with damaged data.

## Loading and calls

Native libraries are resolved before `main` runs: the program opens each
distinct declared library and resolves each declared `SYMBOL` and `FREE`
deallocator.

The executable does not satisfy `LINK` symbols through the OS link editor as
ordinary unresolved externals. The MFBASIC initializer loads them at runtime and
fails before `main` if a required library or symbol is unavailable.

## ABI types

The ABI type names below are the C-facing types an `ABI` signature may use:

| Type | Meaning |
| --- | --- |
| `CInt8`, `CInt16`, `CInt32`, `CInt64` | Signed fixed-width integer slots. |
| `CUInt8`, `CUInt16`, `CUInt32`, `CUInt64` | Unsigned fixed-width integer slots. |
| `CBool` | C Boolean slot. |
| `CFloat`, `CDouble` | 32-bit and 64-bit floating-point slots. |
| `CByte` | C unsigned-byte slot. |
| `CString` | Null-terminated UTF-8 pointer produced from a MFBASIC `String` for the duration of the call. |
| `CPtr` | Opaque native pointer, valid only inside native bindings or as the hidden representation of a declared resource. |
| `CVoid` | Native `void` return, valid only in an ABI return or `FREE` deallocator signature. |

Raw C ABI types may not appear in a wrapper's MFBASIC-facing signature. Exposing
`CPtr`, `CString`, or fixed C integer types as ordinary source API is rejected;
wrap native handles in `RESOURCE` types instead.

## Diagnostics

| Code | Name | Raised when |
| --- | --- | --- |
| `1-102-0008` | `MFB_PARSE_MISSING_NATIVE_SYMBOL` | a native `FUNC` omits `SYMBOL` |
| `1-102-0009` | `MFB_PARSE_MISSING_NATIVE_ABI` | a native `FUNC` omits `ABI` |
| `2-203-0089` | `RESOURCE_CLOSE_NOT_NATIVE` | a resource `CLOSE BY` target is not a native `LINK` function |
| `2-203-0090` | `RESOURCE_CLOSE_MISSING` | a resource names a missing function in a known `LINK` alias |
| `2-203-0091` | `RESOURCE_CLOSE_SIGNATURE` | a close op does not take exactly one `RES` parameter of the resource type |
| `2-203-0092` | `NATIVE_CPTR_ESCAPE` | a raw C ABI type appears outside an ABI slot |
| `2-203-0093` | `NATIVE_ABI_RESULT_MARKER` | a `RETURN` clause is malformed or ambiguous, or a `Nothing` wrapper declares one |
| `2-203-0094` | `NATIVE_ABI_UNBOUND_SLOT` | an input ABI slot is not bound to a parameter or `CONST` pin, or an expression names something that is not a slot |
| `2-203-0095` | `NATIVE_ABI_UNBOUND_PARAM` | a wrapper parameter has no matching ABI slot |
| `2-203-0096` | `NATIVE_ABI_NO_RESULT` | a value-returning native wrapper has no `RETURN` |
| `2-205-0002` | `NATIVE_MANIFEST_INVALID` | imported native binding metadata is malformed or inconsistent |
| `2-203-0097` | `NATIVE_CONST_OUT` | a `CONST`-pinned ABI slot is also `OUT` |
| `2-203-0098` | `NATIVE_CONST_UNKNOWN_SLOT` | a `CONST` pin names an unknown ABI slot |
| `2-203-0099` | `NATIVE_FREE_INVALID` | a `FREE` block is malformed — it must release a produced `CPtr` slot through a deallocator taking one `CPtr` and returning `CVoid` |
| `2-203-0123` | `NATIVE_ABI_UNKNOWN_CTYPE` | an ABI slot or return names a C type the compiler does not support |
| `2-203-0124` | `NATIVE_CSTRUCT_INVALID` | a `CSTRUCT` declaration is not a layout the compiler can compute faithfully |
| `2-203-0125` | `NATIVE_CSTRUCT_TOO_LARGE` | a `CSTRUCT` lays out larger than the maximum native struct size |
| `2-203-0126` | `NATIVE_CSTRUCT_ESCAPE` | a `CSTRUCT` name is used outside its `LINK` block, where only its mapped record type is nameable |
| `2-203-0127` | `NATIVE_STRUCT_FIELD_MISMATCH` | a `CSTRUCT` and the record it maps to differ by field name, type, or coverage |
| `2-203-0128` | `NATIVE_BIND_IN_INVALID` | a `BIND IN` block names an unknown slot or field, or binds a value it cannot marshal |
| `2-203-0132` | `NATIVE_BUFFER_INVALID` | a `CBuffer` slot is not an `OUT` slot with exactly one `BUFFER` clause, named by `RETURN`, on a wrapper returning `List OF Byte` |
| `2-203-0130` | `NATIVE_BIND_STATE_INVALID` | a `BIND STATE` does not name the native function's stateful resource return and an `OUT` `CSTRUCT` slot whose record is the resource's `STATE` type |
| `2-203-0114` | `NATIVE_LIBRARY_MISSING` | a `LINK "name"` has no `libraries` entry in project.json |
| `2-203-0115` | `NATIVE_LIBRARY_TARGET_UNCOVERED` | a supported target has no locator (warn; one per uncovered slot) |
| `2-203-0116` | `NATIVE_LIBRARY_SOURCE_UNREADABLE` | a `vendor` locator's file under `vendor/` is missing or unreadable |
| `2-203-0117` | `NATIVE_LIBRARY_UNUSED` | a `libraries` entry has no matching `LINK` block (warn) |
| `2-203-0118` | `NATIVE_LIBRARY_NO_MATCH` | no locator matches the target being built |
| `2-203-0119` | `NATIVE_LIBRARY_AMBIGUOUS` | two equally-specific locators match the target |
| `2-203-0120` | `NATIVE_LIBRARY_FILE_MISSING` | a resolved `vendor` library is absent from the consumer's `vendor/` |
| `2-203-0121` | `NATIVE_LIBRARY_HASH_MISMATCH` | a resolved `vendor` library is the wrong version (sha256 differs) |
| `2-203-0122` | `NATIVE_LIBRARY_VENDOR_COLLISION` | two declaring units vendor different native libraries that copy to the same output filename |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | a native wrapper is called with a closed resource handle |
| `77030007` | `ErrNativeBindingUnavailable` | the program cannot load a required native library or resolve a required symbol at startup |
| `77030008` | `ErrNativeBindingCallFailed` | a native call fails its `SUCCESS_ON` or `ERROR_ON` gate |
| `77030010` | `ErrNativeBufferOverrun` | a native function writes past a `CBuffer` slot's `SIZE` |

## See also

- `mfb man errors`
- `mfb man types`
- `mfb spec language native-libraries`
- `mfb spec linker import-selection`
- `mfb spec package native-bindings`

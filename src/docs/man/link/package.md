# link

Native LINK declarations for binding host dynamic libraries

## Synopsis

```
mfb man link
```

## Imports

`link` is a developer documentation topic, not an importable package. `LINK`,
`RESOURCE`, `SYMBOL`, `ABI`, `CONST`, `SUCCESS_ON`, `ERROR_ON`, `RESULT`, and
`FREE` are source forms used inside a binding package.

## Description

A `LINK` block declares the native surface of a reusable binding package. It
names one host dynamic library, gives that library a package-local alias, and
declares typed MFBASIC wrapper functions for native symbols in that library.
Application packages do not repeat the `LINK` block. They import the compiled
binding package, call its exported wrappers, and use any exported resource types
through ordinary MFBASIC ownership rules.

```
EXPORT RESOURCE Db CLOSE BY sqlite::close

LINK "sqlite3" AS sqlite
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC close(RES db AS Db) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

```
EXPORT RESOURCE Db CLOSE BY sqlite::close

LINK "sqlite3" AS sqlite
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, return OUT CPtr) AS status CInt32
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
package. The resolver collects `LINK` aliases before ordinary top-level symbols
so resource `CLOSE BY` declarations and transparent re-export aliases can
forward-reference native functions. A `LINK` alias is distinct from an imported
package and wins before import lookup for that root name.
[[src/resolver/mod.rs:collect_top_level_symbols]]
[[src/resolver/resolution.rs:resolve_package_qualified_name]]

## Binding packages

A source package that declares `LINK` is a binding package. It may contain only
the native declarations, or it may add ordinary MFBASIC wrapper code around them
for validation, safer defaults, and higher-level APIs. The compiled `.mfp`
contains normal package metadata plus native binding metadata, so importers see
ordinary package functions and resource types.

Native resources are declared at package scope with
`RESOURCE Name CLOSE BY alias::func`, not inside the `LINK` block. The close
function must be a native `LINK` function that consumes exactly one `RES`
parameter of that resource type. A transparent function alias,
`EXPORT FUNC close AS alias::func`, is the way to expose that same consuming
close operation to importers. [[src/resolver/resolution.rs:resolve_resource_decl]]
[[src/resolver/resolution.rs:resolve_func_alias]]

## Native functions

Each native `FUNC` has two signatures:

- The MFBASIC-facing signature after `FUNC`, using source types such as
  `String`, `Integer`, `Float`, `Nothing`, and `RES Db`.
- The C-facing ABI signature after `ABI`, using ABI slot types such as
  `CString`, `CPtr`, `CInt32`, `CBool`, `CByte`, `CDouble`, and `CVoid`.

`SYMBOL "name"` gives the exact dynamic-library symbol to resolve. `ABI (...)`
lists native arguments in C call order, and `AS slot CType` names the native
return slot. ABI slots bind to wrapper parameters by name. Every wrapper
parameter must have a matching ABI slot, and every ABI slot must be a wrapper parameter, a `CONST` pin, or the wrapper result marker.

## Binding packages

A source package that declares `LINK` is a binding package. It may contain only
the native declarations, or it may add ordinary MFBASIC wrapper code around them
for validation, safer defaults, and higher-level APIs. The compiled `.mfp`
contains normal package metadata plus native binding metadata, so importers see
ordinary package functions and resource types.

Native resources are declared at package scope with
`RESOURCE Name CLOSE BY alias::func`, not inside the `LINK` block. The close
function must be a native `LINK` function that consumes exactly one `RES`
parameter of that resource type. A transparent function alias,
`EXPORT FUNC close AS alias::func`, is the way to expose that same consuming
close operation to importers. [[src/resolver/resolution.rs:resolve_resource_decl]]
[[src/resolver/resolution.rs:resolve_func_alias]]

## Native functions

Each native `FUNC` has two signatures:

- The MFBASIC-facing signature after `FUNC`, using source types such as
  `String`, `Integer`, `Float`, `Nothing`, and `RES Db`.
- The C-facing ABI signature after `ABI`, using ABI slot types such as
  `CString`, `CPtr`, `CInt32`, `CBool`, `CByte`, `CDouble`, and `CVoid`.

`SYMBOL "name"` gives the exact dynamic-library symbol to resolve. `ABI (...)`
lists native arguments in C call order, and `AS slot CType` names the native
return slot. ABI slots bind to wrapper parameters by name. Every wrapper
parameter must have a matching ABI slot, and every ABI slot must be a wrapper
parameter, a `CONST` pin, or the wrapper result marker.

Use `return` as the result slot name for the native return value or for the
single supported `OUT return` slot. A value-returning wrapper must expose exactly
one result with `return` or a `RESULT` expression. The current compiler does not
support multiple `OUT` slots or `RETURN_OUT`; any `OUT` slot other than
`return` is rejected as an unbound ABI slot. [[src/ast/items.rs:parse_link_function]]
[[src/syntaxcheck/mod.rs]]

## Result gates

`SUCCESS_ON` and `ERROR_ON` describe when a native call succeeds. The condition
is a Boolean expression over ABI slot names:

```
SUCCESS_ON status = 0
ERROR_ON status = -1
SUCCESS_ON status = 100 OR status = 101
```

When the gate says the call failed, the wrapper fails with
`ErrNativeBindingCallFailed` and ordinary MFBASIC error propagation applies. A
`RESULT` expression may map ABI slots into the MFBASIC success result. `CONST`
pins provide fixed ABI slot values without exposing them as wrapper parameters.
`FREE return` runs a declared native deallocator after a successful copy from a
caller-owned native pointer into an owned MFBASIC value.
[[src/ir/lower.rs:lower_link_expr]] [[src/ir/lower.rs:eval_link_const]]

## Loading and calls

Native libraries are resolved before `main`. The backend emits
`_mfb_linker_init`, which opens each distinct declared library and resolves each
declared `SYMBOL` and `FREE` deallocator into a global pointer slot. Calls to
native wrappers go through generated marshaling thunks named from the `LINK`
alias and function name. [[src/target/shared/code/link_thunk.rs]]
[[src/target/shared/nir/mod.rs:LINK_INIT_SYMBOL]]
[[src/target/shared/nir/mod.rs:link_thunk_symbol]]

The executable does not satisfy `LINK` symbols through the OS link editor as
ordinary unresolved externals. The MFBASIC initializer loads them at runtime and
fails before `main` if a required library or symbol is unavailable.

## ABI types

The ABI type names below are the names the marshaling backend acts on:

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
wrap native handles in `RESOURCE` types instead. [[src/syntaxcheck/helpers.rs:is_c_abi_type]]

## Diagnostics

| Code | Name | Raised when |
| --- | --- | --- |
| `1-102-0008` | `MFB_PARSE_MISSING_NATIVE_SYMBOL` | a native `FUNC` omits `SYMBOL` |
| `1-102-0009` | `MFB_PARSE_MISSING_NATIVE_ABI` | a native `FUNC` omits `ABI` |
| `2-203-0089` | `RESOURCE_CLOSE_NOT_NATIVE` | a resource `CLOSE BY` target is not a native `LINK` function |
| `2-203-0090` | `RESOURCE_CLOSE_MISSING` | a resource names a missing function in a known `LINK` alias |
| `2-203-0091` | `RESOURCE_CLOSE_SIGNATURE` | a close op does not consume exactly one `RES` parameter of the resource type |
| `2-203-0092` | `NATIVE_CPTR_ESCAPE` | a raw C ABI type appears outside an ABI slot |
| `2-203-0093` | `NATIVE_ABI_RESULT_MARKER` | a result marker is malformed or ambiguous |
| `2-203-0094` | `NATIVE_ABI_UNBOUND_SLOT` | an ABI slot is not bound to a parameter, result marker, or `CONST` pin |
| `2-203-0095` | `NATIVE_ABI_UNBOUND_PARAM` | a wrapper parameter has no matching ABI slot |
| `2-203-0096` | `NATIVE_ABI_NO_RESULT` | a value-returning native wrapper exposes no result |
| `2-205-0002` | `NATIVE_MANIFEST_INVALID` | imported native binding metadata is malformed or inconsistent |
| `2-203-0114` | `NATIVE_LIBRARY_MISSING` | a `LINK "name"` has no `libraries` entry in project.json |
| `2-203-0115` | `NATIVE_LIBRARY_TARGET_UNCOVERED` | a supported target has no locator (warn; one per uncovered slot) |
| `2-203-0116` | `NATIVE_LIBRARY_SOURCE_UNREADABLE` | a `vendor` locator's file under `vendor/` is missing or unreadable |
| `2-203-0117` | `NATIVE_LIBRARY_UNUSED` | a `libraries` entry has no matching `LINK` block (warn) |
| `2-203-0118` | `NATIVE_LIBRARY_NO_MATCH` | no locator matches the target being built |
| `2-203-0119` | `NATIVE_LIBRARY_AMBIGUOUS` | two equally-specific locators match the target |
| `2-203-0120` | `NATIVE_LIBRARY_FILE_MISSING` | a resolved `vendor` library is absent from the consumer's `vendor/` |
| `2-203-0121` | `NATIVE_LIBRARY_HASH_MISMATCH` | a resolved `vendor` library is the wrong version (sha256 differs) |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | a native wrapper is called with a closed resource handle [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77030007` | `ErrNativeBindingUnavailable` | `_mfb_linker_init` cannot load a required native library or resolve a required symbol |
| `77030008` | `ErrNativeBindingCallFailed` | a native call fails its `SUCCESS_ON` or `ERROR_ON` gate |

## See also

- `mfb man errors`
- `mfb man types`
- `mfb spec language native-libraries`
- `mfb spec linker import-selection`
- `mfb spec package native-bindings`

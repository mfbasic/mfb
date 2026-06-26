# Native Binding Metadata

A package containing `LINK` declarations is still a normal `.mfp` package. The application imports it normally. The binding metadata lives inside the signed binary representation payload.

The existing native interface separates MFBASIC-facing wrapper signatures from C-facing ABI signatures, with `CString`, `CPtr`, `OUT`, `REF`, `SUCCESS_ON`, and `ERROR_ON` rules.  The `.mfp` stores those rules so importers do not repeat the `LINK`.

## `NATIVE_LINK_TABLE`

```text
nativeLibraryCount   u32
NativeLibrary[nativeLibraryCount]

nativeSymbolCount    u32
NativeSymbol[nativeSymbolCount]

nativeAbiCount       u32
NativeAbi[nativeAbiCount]
```

## Native library entry

```text
namespaceName        stringId
libraryName          stringId
versionConstraint    stringId
flags                u32
```

Flags:

```text
bit 0 = required
bit 1 = system library allowed
bit 2 = vendored library allowed
bit 3 = current-directory lookup forbidden
bit 4 = thread-safe
```

Current-directory lookup should be forbidden by default.

## Native symbol entry

```text
libraryId            u32
symbolName           stringId
wrapperFunctionId    functionId
abiId                u32
returnRuleKind       u16
returnRuleValue      i64
flags                u32
```

Return rule kinds:

```text
0 = direct return
1 = SUCCESS_ON
2 = ERROR_ON
```

## Native ABI entry

```text
paramCount           u32

repeated paramCount times:
  direction          u16
  nativeType         u16
  sourceType         typeId

returnNativeType     u16
returnSourceType     typeId
returnOutCount       u32
```

Directions:

```text
0 = value
1 = REF
2 = OUT
3 = resource CPtr
```

Native ABI types:

```text
1  = CInt8
2  = CInt16
3  = CInt32
4  = CInt64
5  = CUInt8
6  = CUInt16
7  = CUInt32
8  = CUInt64
9  = CBool
10 = CFloat32
11 = CFloat64
12 = CIntPtr
13 = CUIntPtr
14 = CSize
15 = CString
16 = CPtr
17 = CVoid
```

Rules:

* `CString` conversion rejects embedded NUL.
* `CPtr` may appear only inside native binding metadata.
* `CPtr` must not appear in ordinary exported MFBASIC function signatures.
* `OUT` and `REF` storage lives only for the duration of the call unless explicitly converted into an owned MFBASIC value or resource.
* Native symbols are whitelisted by this table. MFBASIC binary representation cannot perform dynamic native symbol lookup.

## Built-in native runtime helpers

Executable native backends provide stable helper symbols for compiler-owned built-ins. These helpers are not user-callable `LINK` symbols and do not appear in source packages as dependencies. The arch emitter requests symbolic runtime imports; the OS layer supplies or links the implementation.

```text
mfb_io_open(path_ptr, path_len, mode_id) -> err_code, handle
mfb_io_close(handle) -> err_code
```

Runtime helper ABI:

```text
mfb_io_open
  input:
    x0 = UTF-8 path pointer
    x1 = path byte length
    x2 = portable MFB open mode id
  output:
    x0 = MFB error code, 0 on success
    x1 = opaque signed handle on success, unspecified on error

mfb_io_close
  input:
    x0 = opaque signed handle
  output:
    x0 = MFB error code, 0 on success
```

The calling convention is the target platform ABI. Caller-saved and callee-saved registers follow that ABI. Helpers must not unwind or throw exceptions across the MFB frame. `path_ptr` remains owned by the MFB caller/runtime and no heap ownership transfers to the helper. Embedded NUL bytes are rejected before calling OS APIs. Portable open mode IDs are compiler-defined and map to source modes such as read, write, readWrite, and append; OS backends translate them to platform-specific flags.

On macOS AArch64, the OS layer may implement these helpers by linking `/usr/lib/libSystem.B.dylib` and importing `_open`, `_close`, and `___error`, or by emitting equivalent OS-specific helper code. Linux and Windows backends must implement the same helper names and return convention while mapping to libc/syscalls or Win32 handles internally.

## `RESOURCE_TABLE`

```text
resourceCount       u32

repeated resourceCount times:
  resourceType      typeId
  closeFunctionId   functionId
  flags             u32
```

Resource flags:

```text
bit 0 = native resource
bit 1 = standard resource
bit 2 = sendable to thread
bit 3 = close may fail
```

Default rule: resources are not sendable to threads.

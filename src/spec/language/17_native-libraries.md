# 17. Native Libraries

Native libraries are host dynamic libraries loaded through reusable `.mfp` binding packages. MFBASIC code cannot call arbitrary C symbols directly. A binding package introduces its **native resource types at package scope** (`RESOURCE … CLOSE BY …`) and declares a `LINK` block holding the library name, its package-like namespace, and the typed native wrapper functions visible to MFBASIC code. Compiling that package emits a normal `.mfp` (structured Binary Representation) plus native binding metadata.

Application packages do not repeat a dependency's `LINK` block. They import the binding package normally with `IMPORT`, call its exported wrapper functions, and use its resource types through ordinary ownership and lexical-drop behavior. Final executable builds collect native dependencies from all imported `.mfp` packages, resolve them once for the target platform, validate their manifests, and link or load the declared native libraries before `main`. Each library is opened and every declared symbol resolved by a generated load-time initializer that runs before `main`; if a library or symbol cannot be loaded the program aborts at startup with `ErrNativeBindingUnavailable` (`77030007`) rather than continuing with an unbound symbol.

* Native ABI details do not leak across package boundaries unless explicitly part of the binding package's public API.
* Application code importing a binding package sees ordinary MFBASIC types, functions, resources, failure/auto-propagation behavior, and lexical-drop cleanup behavior.
* A source package that declares `LINK` is a binding package. It may also include ordinary MFBASIC wrapper code, validation, and higher-level helpers around the native symbols.

```basic
' The native resource type is declared at PACKAGE scope. `EXPORT` makes it
' nameable by importers as `sqlite::Db`; `CLOSE BY` names its registered close
' op — a native LINK function declared below.
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

' Re-export the registered close op under the package name (see below).
EXPORT FUNC close AS sqlite::close
```

`LINK "sqlite3" AS sqlite` creates the namespace `sqlite`, so native wrapper functions are called like package functions. A producer wrapper returns `AS RES Db`, so its result is bound with `RES`:

```basic
RES db AS Db = sqlite::open("app.db")
' borrow db through wrapper calls...
' db is closed by lexical drop when its scope ends, or by an explicit sqlite::close(db)
```

**Declaring the native resource (`[visibility] RESOURCE Name CLOSE BY closeFn`).** A native resource is declared at **package scope, not inside the `LINK` block** — it is named and exported exactly like any other type, so package wrapper code resolves the bare name `Db` and `EXPORT` lets importers write `sqlite::Db`. Omitting `EXPORT` keeps it package-private. The declaration may forward-reference a `LINK` function defined later in the file. `Db` is an opaque unique native handle whose hidden representation is a `CPtr`; source code cannot inspect, cast, compare, serialize, print, copy, capture in a lambda, store in an ordinary collection, do arithmetic on it, or name its `CPtr`. A resource may be passed only to functions whose signatures explicitly accept that resource type in a `RES` position. Resources are not thread-sendable unless the declaration opts in with a trailing `THREAD_SENDABLE`.

`CLOSE BY <closeFn>` names the resource's **registered close op** — a native `LINK` function whose single `RES` parameter is this resource type (overhaul invalidation event #1). It runs automatically when the resource binding is dropped at scope exit, including on error exits, and may also be called explicitly to release early or to observe a close failure. `closeFn` must be a native `LINK` function; naming an ordinary MFBASIC function is rejected (`RESOURCE_CLOSE_NOT_NATIVE`), and a close op that does not consume exactly one `RES` parameter of its resource is rejected (`RESOURCE_CLOSE_SIGNATURE`). Calling a native wrapper with a closed resource fails with `ErrResourceClosed`.

**Re-exporting the close op (`[visibility] FUNC alias AS qualified::func`).** A binding publishes its close op under the package name with a transparent **function alias**, so importers can close explicitly through `sqlite::close(db)`. The alias is the *same* registered close op: calling it consumes its `RES` argument exactly as the native close op does. A hand-written wrapper `FUNC close(RES db AS Db) … sqlite::close(db)` cannot replace it, because its parameter is a borrow and a borrow may never invalidate (§15) — there is no `MOVE` annotation. The alias form is required for any close op importers should be able to call.

`SYMBOL "sqlite3_open"` gives the exact native symbol name to look up in the loaded library. The MFBASIC function name is the public wrapper name; it does not have to match the native symbol name.

`ABI (...) AS ...` gives the native C-facing call shape. The `FUNC` signature is the MFBASIC-facing wrapper type; the `ABI` signature is the host-library symbol's argument and return representation. Each ABI slot is `name type` in native C argument order, and slots bind to wrapper parameters **by name** (so `path` in the ABI matches the `path` parameter). One slot may be named `return` to mark the wrapper's result (an `OUT` slot for a produced handle/value, or the native return slot after `AS`). Every wrapper parameter must map to an ABI slot of the same name, and every ABI slot must be satisfied by exactly one of: a wrapper parameter, the `return` result marker, or a `CONST` pin — otherwise `NATIVE_ABI_UNBOUND_PARAM` / `NATIVE_ABI_UNBOUND_SLOT`.

Native ABI types are separate from MFBASIC source types:

| Type | Meaning |
|------|---------|
| `CInt8`, `CInt16`, `CInt32`, `CInt64` | Signed fixed-width C integer values. |
| `CUInt8`, `CUInt16`, `CUInt32`, `CUInt64` | Unsigned fixed-width C integer values. |
| `CBool` | C `_Bool` / `bool` value. |
| `CFloat32`, `CFloat64` | 32-bit and 64-bit C floating-point values. |
| `CIntPtr`, `CUIntPtr` | Signed and unsigned integer values with pointer width. |
| `CSize` | Unsigned C size value, equivalent to `size_t`. |
| `CString` | Null-terminated UTF-8 string pointer created from a MFBASIC `String` for the duration of the call. Embedded NUL bytes are rejected before the native call with `ErrInvalidArgument` (`77050002`). |
| `CPtr` | Opaque native pointer value used only inside native bindings. It cannot be inspected, manipulated, stored, returned, or named by ordinary MFBASIC code except as the hidden representation of a declared `RESOURCE`. |
| `CVoid` | Native `void` return. Valid only as an ABI return type. Use MFBASIC `Nothing` for the wrapper's source-level return type. |

The fixed-width names are preferred over C spellings such as `int` or `long`, because those spellings vary by platform. Bindings should map the platform header's actual ABI to one of the fixed or pointer-sized types.

The marshaling boundary validates values rather than silently corrupting them: an `Integer` argument that does not fit a narrower signed C integer fails with `ErrOverflow` (`77050010`) instead of truncating; a C floating-point **return** that is NaN or infinite is rejected with `ErrFloatNaN` (`77050013`) / `ErrFloatInf` (`77050014`), since an MFBASIC `Float` is always finite (§3); and the bytes of a returned C string are validated as UTF-8, failing with `ErrEncoding` (`77020004`) when malformed.

ABI parameters may use direction modifiers:

| Form | Meaning |
|------|---------|
| `REF T` | Pass a pointer to a temporary native value initialized from the MFBASIC argument. The pointer lifetime ends when the native call returns. |
| `OUT T` | Pass a pointer to uninitialized native storage and copy the result back after the call. The pointer lifetime ends when the native call returns. |
| `CPtr` | Pass a resource handle or opaque pointer as-is inside the binding boundary. |

**Pinning constant and NULL arguments (`CONST slot = value`).** The `ABI (...)` line always states the true native signature — every C argument in C order. Some of those arguments are fixed values the caller never supplies (a `-1` length, a NULL callback, a sentinel destructor). `CONST <slot> = <value>` pins one ABI slot to a fixed value and removes it from the wrapper's parameter list. The value is checked against the slot's declared ABI type. `NOTHING` pins a C NULL on a pointer slot; a pointer-sized integer literal pins a sentinel pointer (e.g. `-1` for SQLite's `SQLITE_TRANSIENT`). A `CONST` slot is input-only — marking it `OUT` or as the result is rejected (`NATIVE_CONST_OUT`), and pinning an unknown slot is `NATIVE_CONST_UNKNOWN_SLOT`. A pin is call metadata baked into the native frame; it never materializes as a source value, so it cannot forge or leak a `CPtr`.

```basic
FUNC bindText(RES stmt AS Stmt, iCol AS Integer, zVal AS String) AS Nothing
  SYMBOL "sqlite3_bind_text"
  ABI (stmt CPtr, iCol CInt32, zVal CString, nByte CInt32, destructor CPtr) AS status CInt32
  CONST nByte = -1            ' bind up to the terminating NUL
  CONST destructor = -1       ' SQLITE_TRANSIENT (void*)-1: copy the bytes now
  SUCCESS_ON status = 0
END FUNC
```

**Success gating (`SUCCESS_ON` / `ERROR_ON`).** When the native return is a status code rather than the result, a Boolean expression over the named slots decides success:

| Form | Meaning |
|------|---------|
| `SUCCESS_ON <expr>` | The wrapper succeeds when `<expr>` is true; any other outcome auto-propagates as an `Error`. |
| `ERROR_ON <expr>` | The De Morgan complement of `SUCCESS_ON`; the wrapper fails when `<expr>` is true. A wrapper states one, not both. |

`<expr>` is any Boolean expression over slot names, including compound conditions: `SUCCESS_ON status = 0`, `SUCCESS_ON status >= 0`, or `SUCCESS_ON status = 100 OR status = 101`. Comparisons bind tighter than `AND`/`OR` (§11), so the compound form needs no parentheses. `SUCCESS_ON status = 0` is common for libraries such as SQLite; `ERROR_ON status = -1` is common for POSIX-style APIs. When the gate fails, the wrapper produces `ErrNativeBindingCallFailed` (`77030008`), which auto-propagates like any other call failure.

**Result value mapping (`RESULT <expr>`).** When the wrapper's result is *derived from* the status (rather than passed straight through or produced via `OUT`), `RESULT <expr>` supplies it. For example SQLite's `sqlite3_step` returns `SQLITE_ROW` (100) or `SQLITE_DONE` (101); the wrapper returns `AS Boolean` meaning "a row is ready":

```basic
FUNC step(RES stmt AS Stmt) AS Boolean
  SYMBOL "sqlite3_step"
  ABI (stmt CPtr) AS status CInt32
  SUCCESS_ON status = 100 OR status = 101   ' both are non-errors
  RESULT status = 100                       ' TRUE iff a row is ready
END FUNC
```

A plain value-returning call needs neither gate nor mapping: name the native return slot `return` and the C return becomes the wrapper's result (e.g. `ABI (stmt CPtr, name CString) AS return CInt32`). A value-producing wrapper that marks no result (`return` / `RESULT`) is rejected (`NATIVE_ABI_NO_RESULT`).

**Multiple outputs (`RETURN_OUT`).** When an ABI signature has more than one `OUT` slot, `RETURN_OUT` defines how those outputs become the success value, referencing slots by name. A single `OUT` slot named `return` is returned implicitly.

```basic
TYPE DivModResult
  quotient AS Integer
  remainder AS Integer
END TYPE

LINK "mylib" AS mylib
  FUNC divmod(a AS Integer, b AS Integer) AS DivModResult
    SYMBOL "divmod"
    ABI (a CInt32, b CInt32, quotient OUT CInt32, remainder OUT CInt32) AS CVoid
    RETURN_OUT DivModResult[quotient, remainder]
  END FUNC
END LINK
```

`RETURN_OUT DivModResult[quotient, remainder]` means: after the native call succeeds, read the named `OUT` slots and succeed with `DivModResult[quotient, remainder]`.

**Freeing a caller-owned return (`FREE`).** A `CPtr` result mapped to an owned MFBASIC value (such as `AS String`) is **copied** out of the native buffer and the source pointer is then left untouched — *copy-and-leave*. That is correct when the native library **owns** the buffer and keeps it valid (a transient or static pointer), as with `sqlite3_column_text`. When the call instead returns a buffer the **caller owns and must release** — `sqlite3_expanded_sql`, `sqlite3_mprintf`, `strdup` — copy-and-leave would leak it. A `FREE` block names the produced slot and the deallocator that releases it:

```basic
LINK "sqlite3" AS sqlite
  FUNC expandedSql(RES stmt AS Stmt) AS String
    SYMBOL "sqlite3_expanded_sql"
    ABI (stmt CPtr) AS return CPtr
    FREE return
      SYMBOL "sqlite3_free"
      ABI (ptr CPtr) AS CVoid
    END FREE
  END FUNC
END LINK
```

`FREE return` means: after the wrapper has copied the `return` slot into its owned MFBASIC result, pass the **original** native pointer to the named deallocator. The nested `SYMBOL`/`ABI` declare that deallocator — exactly one pointer parameter and a `CVoid` return. The freed slot is the produced pointer: the C `return`, or a named `OUT` slot. The deallocator runs **once, after the copy, on the success path only**; if the wrapper fails before the value is produced (a failed `SUCCESS_ON` gate, a marshaling error), nothing is freed. A NULL produced pointer is passed to the deallocator unchanged, because deallocators such as `sqlite3_free` define NULL as a no-op. The original pointer is never surfaced as an MFBASIC value, so `FREE` is the only sanctioned way to release a caller-owned native return — a raw `CPtr` cannot be handed back to source code to free by hand (`NATIVE_CPTR_ESCAPE`). A binding with more than one caller-owned pointer (for example several `OUT` buffers) states one `FREE` block per slot.

Rules:

- `LINK` names and all declared `SYMBOL` names are resolved before `main` starts. Native libraries are not lazy-loaded.
- If a required native library or symbol cannot be loaded before `main`, the program terminates before entering `main`. The diagnostic is written to stderr and the process exits with `55000001` (`ErrLinkFailed`). This startup failure is outside the error/`TRAP` model because no MFBASIC function is running yet.
- Linked names occupy a package-like namespace. A package-qualified name such as `sqlite::open` follows the same two-part rule as package access.
- A native call may resolve only the symbols declared by `SYMBOL` entries in the binding package. Dynamic lookup by source strings or computed names is not available to ordinary MFBASIC code.
- Native functions expose ordinary MFBASIC signatures. At call sites they auto-unwrap, auto-propagate, and participate in `MATCH` like any other fallible function.
- Native functions may accept and return MFBASIC primitive values, strings, byte lists, and declared resource types through an explicit `ABI` mapping. Other conversions are implementation-defined unless specified by the binding.
- If a native function has more than one `OUT` parameter and its MFBASIC return type is not `Nothing`, it must declare `RETURN_OUT`.
- A `FREE` block must name a `CPtr`-typed produced slot — the `return` slot or a declared `OUT` slot — and its deallocator must declare exactly one pointer parameter and a `CVoid` return. The deallocator is called once on the success path, after the produced value is copied into the wrapper's owned MFBASIC result, with the original (possibly NULL) native pointer; it is not called on a failed call. Without a `FREE` block a `CPtr` result is copied and the source pointer is left untouched (copy-and-leave), which leaks a caller-owned buffer — `FREE` is the only way to release one.
- `RESOURCE` is a declaration form for concrete opaque unique-handle types; it is not an inheritance base type and cannot be used as a generic catch-all type.
- Native resource ownership is declared at package scope with `RESOURCE <Name> CLOSE BY <closeFn>`. Raw C ABI types (`CPtr`, `CString`, `CInt32`, …) may appear only inside `ABI (...)` slots, never in a wrapper's MFBASIC-facing signature; a `CPtr` exists solely as the hidden representation of a declared resource and must not escape into an ordinary API (`NATIVE_CPTR_ESCAPE`).
- `REF` and `OUT` native pointer values are temporary call-frame values. Native code must not retain them after return; if a binding needs retained native storage, it must model that storage as a declared `RESOURCE`.
- Native `LINK` resources slot into the resource model of §15 unchanged: bound with `RES`, borrowed at ordinary calls, auto-closed by lexical drop through the registered close op, never copied/stored/field-accessed, and thread-sendable only with `THREAD_SENDABLE`. Diagnostics specific to native bindings are listed in `specifications/error_codes.md` (`1-102-0008`…`0009`, `2-203-0089`…`0098`).
- Native libraries are platform-specific dependencies. A `.mfp` package may declare that it needs a native library, including version, search policy, platform constraints, and content/hash requirements, but the native library itself is not portable binary representation.

**Example:**

```

EXPORT RESOURCE Db CLOSE BY sqliteLink::close
RESOURCE Stmt CLOSE BY sqliteLink::finalize

LINK "sqlite3" AS sqliteLink
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC openV2(path AS String, flags AS Integer) AS RES Db
    SYMBOL "sqlite3_open_v2"
    ABI (path CString, return OUT CPtr, flags CInt32, zVfs CPtr) AS status CInt32
    CONST zVfs = NOTHING         ' NULL: use the default VFS
    SUCCESS_ON status = 0
  END FUNC

  FUNC close(RES db AS Db) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC busyTimeout(RES db AS Db, ms AS Integer) AS Nothing
    SYMBOL "sqlite3_busy_timeout"
    ABI (db CPtr, ms CInt32) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC exec(RES db AS Db, sql AS String) AS Nothing
    SYMBOL "sqlite3_exec"
    ABI (db CPtr, sql CString, callback CPtr, arg CPtr, errmsg CPtr) AS status CInt32
    CONST callback = NOTHING     ' NULL: no per-row callback
    CONST arg = NOTHING          ' NULL: no callback argument
    CONST errmsg = NOTHING       ' NULL: report failures through status, not a buffer
    SUCCESS_ON status = 0
  END FUNC

  FUNC prepare(RES db AS Db, sql AS String) AS RES Stmt
    SYMBOL "sqlite3_prepare_v2"
    ABI (db CPtr, sql CString, nByte CInt32, return OUT CPtr, pzTail CPtr) AS status CInt32
    CONST nByte = -1             ' read sql up to the terminating NUL
    CONST pzTail = NOTHING       ' NULL: discard the trailing-SQL pointer
    SUCCESS_ON status = 0
  END FUNC

  FUNC bindText(RES stmt AS Stmt, iCol AS Integer, zVal AS String) AS Nothing
    SYMBOL "sqlite3_bind_text"
    ABI (stmt CPtr, iCol CInt32, zVal CString, nByte CInt32, destructor CPtr) AS status CInt32
    CONST nByte = -1             ' bind up to the terminating NUL
    CONST destructor = -1        ' SQLITE_TRANSIENT (void*)-1: copy bytes now, do not alias
    SUCCESS_ON status = 0
  END FUNC

  FUNC bindParameterIndex(RES stmt AS Stmt, name AS String) AS Integer
    SYMBOL "sqlite3_bind_parameter_index"
    ABI (stmt CPtr, name CString) AS return CInt32
  END FUNC

  FUNC step(RES stmt AS Stmt) AS Boolean
    SYMBOL "sqlite3_step"
    ABI (stmt CPtr) AS status CInt32
    SUCCESS_ON status = 100 OR status = 101
    RESULT status = 100
  END FUNC

  FUNC columnText(RES stmt AS Stmt, col AS Integer) AS String
    SYMBOL "sqlite3_column_text"
    ABI (stmt CPtr, col CInt32) AS return CPtr
  END FUNC

  FUNC columnType(RES stmt AS Stmt, col AS Integer) AS Integer
    SYMBOL "sqlite3_column_type"
    ABI (stmt CPtr, col CInt32) AS return CInt32
  END FUNC

  FUNC columnInt(RES stmt AS Stmt, col AS Integer) AS Integer
    SYMBOL "sqlite3_column_int64"
    ABI (stmt CPtr, col CInt32) AS return CInt64
  END FUNC

  FUNC columnDouble(RES stmt AS Stmt, col AS Integer) AS Float
    SYMBOL "sqlite3_column_double"
    ABI (stmt CPtr, col CInt32) AS return CDouble
  END FUNC

  FUNC columnCount(RES stmt AS Stmt) AS Integer
    SYMBOL "sqlite3_column_count"
    ABI (stmt CPtr) AS return CInt32
  END FUNC

  FUNC finalize(RES stmt AS Stmt) AS Nothing
    SYMBOL "sqlite3_finalize"
    ABI (stmt CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC expandedSql(RES stmt AS Stmt) AS String
    SYMBOL "sqlite3_expanded_sql"
    ABI (stmt CPtr) AS return CPtr
    FREE return
      SYMBOL "sqlite3_free"
      ABI (return CPtr) AS CVoid
    END FREE
  END FUNC

  FUNC errmsg(RES db AS Db) AS String
    SYMBOL "sqlite3_errmsg"
    ABI (db CPtr) AS return CPtr
  END FUNC

  FUNC extendedErrcode(RES db AS Db) AS Integer
    SYMBOL "sqlite3_extended_errcode"
    ABI (db CPtr) AS return CInt32
  END FUNC

  FUNC errstr(code AS Integer) AS String
    SYMBOL "sqlite3_errstr"
    ABI (code CInt32) AS return CPtr
  END FUNC
END LINK
```

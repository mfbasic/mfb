# 17. Native Libraries

Native libraries are host dynamic libraries loaded through reusable `.mfp` binding packages. MFBASIC code cannot call arbitrary C symbols directly. A binding package introduces its **native resource types at package scope** (`RESOURCE … CLOSE BY …`) and declares a `LINK` block holding the library name, its package-like namespace, and the typed native wrapper functions visible to MFBASIC code. Compiling that package emits a normal `.mfp` (structured Binary Representation) plus native binding metadata.

Application packages do not repeat a dependency's `LINK` block. They import the binding package normally with `IMPORT`, call its exported wrapper functions, and use its resource types through ordinary ownership and lexical-drop behavior. Final executable builds collect native dependencies from all imported `.mfp` packages, resolve them once for the target platform, validate their manifests, and link or load the declared native libraries before `main`. Each library is opened and every declared symbol resolved by a generated load-time initializer that runs before `main`; if a library or symbol cannot be loaded the program aborts at startup with `ErrNativeBindingUnavailable` (`77030007`) rather than continuing with an unbound symbol.

* Native ABI details do not leak across package boundaries unless explicitly part of the binding package's public API.
* Application code importing a binding package sees ordinary MFBASIC types, functions, resources, failure/auto-propagation behavior, and lexical-drop cleanup behavior.
* A source package that declares `LINK` is a binding package. It may also include ordinary MFBASIC wrapper code, validation, and higher-level helpers around the native symbols.

```basic
' The native resource type is declared at PUBLIC scope. `EXPORT` makes it
' nameable by importers as `sqlite::Db`; `CLOSE BY` names its registered close
' op — a native LINK function declared below.
EXPORT RESOURCE Db CLOSE BY sqlite::close

LINK "sqlite3" AS sqlite
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, db OUT CPtr) AS status CInt32
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
' pass db to wrapper calls — each receives a pointer to the one handle...
' db is closed by lexical drop when its scope ends, or by an explicit sqlite::close(db)
```

**Declaring the native resource (`[visibility] RESOURCE Name CLOSE BY closeFn`).** A native resource is declared at **package scope, not inside the `LINK` block** — it is named and exported exactly like any other type, so package wrapper code resolves the bare name `Db` and `EXPORT` lets importers write `sqlite::Db`. Omitting `EXPORT` keeps it package-private. The declaration may forward-reference a `LINK` function defined later in the file. `Db` is an opaque unique native handle whose hidden representation is a `CPtr`; source code cannot inspect, cast, compare, serialize, print, copy, capture in a lambda, store in an ordinary collection, do arithmetic on it, or name its `CPtr`. A resource may be passed only to functions whose signatures explicitly accept that resource type in a `RES` position. Resources are not thread-sendable unless the declaration opts in with a trailing `THREAD_SENDABLE`.

The declared name may not reuse a **built-in resource type**'s name — `File`, `Socket`, `Listener`, `UdpSocket`, `TlsSocket`, `TlsListener`, `AudioInput`, `AudioOutput` — which is rejected with `RESOURCE_SHADOWS_BUILTIN`. A user resource does not shadow a built-in: the name would denote the user's type for resolution while the built-in's registered close op still governs which runtime helper the module links, so the two meanings coexist and the build fails downstream with an internal error naming a helper the program never mentions. The rejection is unconditional across all eight names, including where the helper happens to be elided today, since that case is only latently broken until an unrelated import brings the helper back.[[src/syntaxcheck/mod.rs:check_resource_decl]]

`CLOSE BY <closeFn>` names the resource's **registered close op** — a native `LINK` function whose single `RES` parameter is this resource type (resource invalidation event #1, §15). It runs automatically when the resource binding is dropped at scope exit, including on error exits, and may also be called explicitly to release early or to observe a close failure. `closeFn` must be a native `LINK` function; naming an ordinary MFBASIC function is rejected (`RESOURCE_CLOSE_NOT_NATIVE`), and a close op that does not consume exactly one `RES` parameter of its resource is rejected (`RESOURCE_CLOSE_SIGNATURE`). Calling a native wrapper with a closed resource fails with `ErrResourceClosed`.

**Re-exporting the close op (`[visibility] FUNC alias AS qualified::func`).** A binding publishes its close op under the package name with a transparent **function alias**, so importers can close explicitly through `sqlite::close(db)`. The alias is the *same* registered close op: calling it consumes its `RES` argument exactly as the native close op does. A hand-written wrapper `FUNC close(RES db AS Db) … sqlite::close(db)` cannot replace it, because its parameter is a non-owning pointer, and only the owning scope may invalidate a resource (§15) — there is no `MOVE` annotation. The alias form is required for any close op importers should be able to call.

**LINK-alias namespace resolution.** `LINK "lib" AS alias` registers `alias` as a **package-local namespace** — distinct from a package import — whose members resolve directly against the block's declared native functions. The resolver collects every `LINK` block in a dedicated first pass (before all other top-level symbols), keying a per-alias table on the block's alias and mapping each `FUNC` name to its captured signature (parameter types, per-parameter `RES` flags, and return). This first pass runs ahead of ordinary symbol collection precisely so that resource `CLOSE BY` targets and `FUNC … AS` re-export aliases, both of which name `alias::func`, can be resolved.[[src/resolver/mod.rs:collect_top_level_symbols]]

A qualified name `alias::func` is resolved by splitting off the leading root: if the root is a known `LINK` alias, the name resolves *there* — the member after the root must be one of the block's declared native functions, otherwise it is rejected with `SYMBOL_UNKNOWN_IDENTIFIER` ("LINK `alias` does not declare a native function `func`"). This LINK-alias branch is checked **before** the import table, so a LINK alias never falls through to package-import or built-in-package resolution; a root that is not a LINK alias and not an imported package is `SYMBOL_UNKNOWN_IMPORT`. (Although source spells these names with `::`, the resolver operates on the dotted internal form `alias.func`.)[[src/resolver/resolution.rs:resolve_package_qualified_name]]

The same per-alias table backs the two places that must name a native LINK function:

* A resource's `CLOSE BY <closeFn>`. `closeFn` must be the dotted form `alias.func`; a bare name (no alias) is `RESOURCE_CLOSE_NOT_NATIVE`. An unknown alias is also `RESOURCE_CLOSE_NOT_NATIVE` ("references unknown LINK alias"); a known alias that does not declare the named function is `RESOURCE_CLOSE_MISSING`. The close-op signature rule is enforced here: the target must take **exactly one** parameter, that parameter must be marked `RES`, and its base resource type must equal the declaring resource's name — otherwise `RESOURCE_CLOSE_SIGNATURE`.[[src/resolver/resolution.rs:resolve_resource_decl]]
* A re-export `FUNC alias AS <target>`. The `target` must resolve through the same `alias.func` lookup; if it does not name a native LINK function the alias is rejected with `SYMBOL_UNKNOWN_IDENTIFIER` ("targets `…`, which is not a native LINK function"). At symbol-collection time the alias is registered as an ordinary callable carrying the LINK target's parameter types, which is why importers call it like any package function.[[src/resolver/resolution.rs:resolve_func_alias]]

**Declaring a C struct (`CSTRUCT <CName> AS <MfbType>`).** A C function that takes a struct by pointer needs that struct's exact byte layout. `CSTRUCT` declares it inside the `LINK` block, together with the ordinary MFBASIC record it presents as:

```basic
TYPE AudioFormat            ' the public face — an ordinary record
  format    AS Integer
  name      AS String
  extension AS String
END TYPE

LINK "libsnd" AS sndLink
  CSTRUCT SfFormatInfo AS AudioFormat     ' 24 bytes, align 8
    format     CInt32                     ' @0, then 4 bytes of padding
    name       CString                    ' @8
    extension  CString                    ' @16
  END CSTRUCT
END LINK
```

Fields are `name ctype`, one per line, in **C declaration order** — the order is load-bearing, since it drives the offsets, unlike a `TYPE`'s field order.

**The layout is computed, never declared.** There is no offset, size, or padding syntax: the compiler derives each field's offset from the field ctypes using standard C natural alignment (align the running offset up to the field's alignment, place it, advance by its size; the struct's alignment is its widest field's, and the total size is padded out to that). This is not merely convenient — it is what makes the package path safe. Offsets and sizes are **never transported in the `.mfp`**; only the field ctypes are, and the layout is recomputed at decode. A crafted package can therefore choose ctypes, each of which has a known size and alignment, but it has no offset to forge. [[src/ir/link.rs:compute_c_layout]]

A `CSTRUCT`'s field ctype sizes and alignments: 1 byte for `CInt8`/`CUInt8`/`CBool`/`CByte`, 2 for `CInt16`/`CUInt16`, 4 for `CInt32`/`CUInt32`/`CFloat`, and 8 for `CInt64`/`CUInt64`/`CDouble`/`CPtr`/`CString`. A `CString` field is a `const char *` — the **pointer**, not its bytes. `CVoid` has no storage and cannot be a field. Every supported target is LP64 and agrees on this table. [[src/ir/link.rs:ctype_size_align]]

`AS <MfbType>` is required, and names the record this struct presents as. The correspondence is by field name and must be **total**: every `CSTRUCT` field must appear in the record with a compatible type, and vice versa. A silently-unmapped field would be zeroed going in and dropped coming out — a wrong answer with no diagnostic — so partial coverage is rejected (`NATIVE_STRUCT_FIELD_MISMATCH`).

**A `CSTRUCT` name never leaves its `LINK` block.** It is a native-side layout descriptor, not a type, and is nameable in exactly three places: its own declaration, an `ABI (...)` slot's ctype position, and `SIZEOF`. Naming one in a wrapper's MFBASIC-facing signature is `NATIVE_CSTRUCT_ESCAPE` — the same containment argument that confines `CPtr`, and the reason `AS <MfbType>` exists: with the mapping declared once, no other site needs the C name. Ordinary code only ever sees the record.

Rejected as unlayoutable (`NATIVE_CSTRUCT_INVALID`): a struct with no fields, a duplicate field name, a duplicate `CSTRUCT` name within one alias, a `CVoid` field, or a field whose type names another `CSTRUCT` — nested structs, fixed-size arrays, unions, and bitfields are not supported, and there is no packing control. A struct laying out larger than **1024 bytes** is `NATIVE_CSTRUCT_TOO_LARGE`: the struct buffer lives in the marshaling thunk's stack frame, so an unbounded size decoded from a crafted `.mfp` would be a frame-overflow primitive.

`SYMBOL "sqlite3_open"` gives the exact native symbol name to look up in the loaded library. The MFBASIC function name is the public wrapper name; it does not have to match the native symbol name.

`ABI (...) AS ...` gives the native C-facing call shape. The `FUNC` signature is the MFBASIC-facing wrapper type; the `ABI` signature is the host-library symbol's argument and return representation. Each ABI slot is `name type` in native C argument order, and slots bind to wrapper parameters **by name** (so `path` in the ABI matches the `path` parameter). The native return is named too, after `AS`. Every slot name is an ordinary identifier — none is magic. Every wrapper parameter must map to an ABI slot of the same name, and every ordinary (input) ABI slot must be satisfied by a wrapper parameter or a `CONST` pin — otherwise `NATIVE_ABI_UNBOUND_PARAM` / `NATIVE_ABI_UNBOUND_SLOT`. An `OUT` slot needs neither: it is native storage the callee fills, surfaced by naming it in `RETURN`.

Native ABI types are separate from MFBASIC source types. The names below are the ones the marshaling backend acts on, and they are the **only** names an `ABI (...)` slot or return may use: a name outside this table is rejected with `NATIVE_ABI_UNKNOWN_CTYPE`, on both the source path and the `.mfp` package path. Two of them are position-restricted — `CVoid` is valid only as the ABI return (a C function takes no `void` argument), and `CString` only as an argument (it means "build a NUL-terminated copy of this `String` for the duration of the call"; a C function returning `char *` is declared `CPtr` and paired with a wrapper `AS String`, which drives the copy-out). An `OUT` slot is a produced value and so takes a return-shaped ctype. (A separate, narrower allow-list governs which C-ABI type names are rejected from a wrapper's MFBASIC-facing signature via `NATIVE_CPTR_ESCAPE`; it does **not** include `CBool`, `CByte`, or `CVoid`, and it answers the opposite question — do not confuse the two.) [[src/ir/link.rs:abi_slot_ctype_is_known]] [[src/ir/link.rs:abi_ctype_valid_as_return]] [[src/syntaxcheck/helpers.rs:is_c_abi_type]]

| Type | Meaning |
|------|---------|
| `CInt8`, `CInt16`, `CInt32`, `CInt64` | Signed fixed-width C integer values. |
| `CUInt8`, `CUInt16`, `CUInt32`, `CUInt64` | Unsigned fixed-width C integer values. |
| `CBool` | C `_Bool` / `bool` value (return marshals a nonzero value to `TRUE`, zero to `FALSE`). |
| `CFloat`, `CDouble` | 32-bit (`float`) and 64-bit (`double`) C floating-point values. |
| `CByte` | C `unsigned char` byte value (return marshals the low 8 bits). |
| `CString` | Null-terminated UTF-8 string pointer created from a MFBASIC `String` for the duration of the call. |
| `CPtr` | Opaque native pointer value used only inside native bindings. It cannot be inspected, manipulated, stored, returned, or named by ordinary MFBASIC code except as the hidden representation of a declared `RESOURCE`. |
| `CVoid` | Native `void` return. Valid only as an ABI return type (and the `FREE` deallocator return). Use MFBASIC `Nothing` for the wrapper's source-level return type. |
| `CBuffer` | A runtime-sized writable byte buffer the callee fills, surfaced as `List OF Byte`. Valid **only** as an `OUT` slot carrying a `BUFFER … SIZE` clause and named by `RETURN`; see below. |

The fixed-width names are preferred over C spellings such as `int` or `long`, because those spellings vary by platform. Bindings should map the platform header's actual ABI to one of these types.

> Implementation status: the spellings `CFloat32`/`CFloat64`, `CIntPtr`, `CUIntPtr`, and `CSize` do **not** exist — use `CFloat`/`CDouble` and an explicit fixed-width integer instead. Writing one is rejected with `NATIVE_ABI_UNKNOWN_CTYPE`. (Before plan-50-A the slot ctype was an unvalidated free identifier, so such a name compiled clean and silently marshaled as a raw 64-bit register value — usually wrong for floats and narrow integers.) `CFloat` is accepted but is currently marshaled through the raw 64-bit paths rather than as a 32-bit `float`; prefer `CDouble` until that gap is closed.

The marshaling boundary aims to validate values rather than silently corrupting them. As implemented:

* A `CInt32` **argument** is range-checked: a 64-bit MFBASIC `Integer` that does not fit signed 32-bit fails with `ErrOverflow` (`77050010`) instead of truncating. The argument loop special-cases only `CString` and `CInt32`; every other ABI argument type — the narrower integers (`CInt8`/`CInt16`, the `CUInt*` family), `CBool`, `CByte`, and `CPtr` — is passed through as a raw 64-bit value with no narrowing or validation. (`CDouble` arguments are loaded into a floating-point register but otherwise unmodified.)
* A `CDouble` **return** that is NaN or infinite is rejected with `ErrFloatNaN` (`77050013`) / `ErrFloatInf` (`77050014`), since an MFBASIC `Float` is always finite (§3). `CBool`/`CByte`/`CInt32` **returns** are normalized (nonzero→`TRUE`, low-8-bits, sign-extend respectively); other return types load the raw 64-bit value.
* The bytes of a returned `CPtr`-to-`String` are validated as UTF-8, failing with `ErrEncoding` (`77020004`) when malformed; a NULL return yields an empty `String`. [[src/target/shared/code/link_thunk.rs:emit_copy_cstring_to_string]]
* Embedded NUL bytes in a `CString` **argument** are *intended* to be rejected with `ErrInvalidArgument` (`77050002`), but the current marshaling copies the string bytes verbatim without scanning for an interior NUL, so this check is not yet enforced. [[src/target/shared/code/link_thunk.rs:emit_copy_string_to_cstring]]

**Passing a struct (`INOUT` / `BIND IN`).** An `ABI (...)` slot may name a `CSTRUCT` as its ctype. The thunk stages a sized, aligned buffer in its frame, **zeroes it entirely**, writes the bound input fields, and passes its *address* as the C argument — the same shape as an `OUT` scalar, only sized.

```basic
FUNC getFormat(index AS Integer) AS AudioFormat
  SYMBOL "sf_command"
  ABI (sndfile CPtr, command CInt32, info INOUT SfFormatInfo, datasize CInt32) AS status CInt32
  CONST sndfile = NOTHING
  CONST command = 4129                   ' SFC_GET_SIMPLE_FORMAT
  CONST datasize = SIZEOF SfFormatInfo
  BIND IN info
    format = index                       ' the one input field; the rest stay zero
  END BIND
  RETURN info                            ' the post-call struct, as an AudioFormat
  SUCCESS_ON status = 0
END FUNC
```

A struct slot takes a direction: `IN` (the default) for a struct the callee only reads, `OUT` for one it only fills, `INOUT` for both. `INOUT` on a non-struct slot is rejected — a scalar is either an argument or a produced value, never both. The buffer is **always fully zeroed** before the call: that is a correctness requirement (libsndfile demands a zeroed `SF_INFO` for a non-RAW read) and a safety one — an unzeroed buffer would hand the thunk's stack contents to the C library.

`BIND IN <slot> … END BIND` writes named fields before the call; every field it does not name is zero. A value is a wrapper parameter or an integer literal. This is why the caller writes `getFormat(3)` rather than constructing a whole record whose other fields the C function immediately overwrites. `BIND IN` on an `OUT` slot is rejected (the callee fills it), as is naming a field the `CSTRUCT` does not declare, or binding a parameter that does not exist (`NATIVE_BIND_IN_INVALID`).

A parameter consumed by `BIND IN` needs no ABI slot of its own — it feeds a struct *field*. Likewise an `IN` struct slot needs no parameter: its `BIND IN` block satisfies it.

**Integer-slot limit (target-dependent).** A `LINK` thunk calls a real C function, so its arguments follow the target's **external** C ABI, not the compiler's internal calling convention. AArch64 (AAPCS64) and riscv64 pass eight integer arguments in registers; SysV x86-64 passes only **six**, taking the rest from the stack. Stack arguments are not yet staged for native calls, so a native function declaring more integer (non-`CDouble`) ABI slots than the target passes in registers is **rejected at build time** rather than called with arguments the callee never reads. In practice this means at most six integer slots for `linux-x86_64` and eight elsewhere; `CDouble` slots use a separate float register bank and do not count against it. [[src/target/shared/regmodel.rs:external_int_argument_registers]] (Before bug-296 the x86 backend silently passed the 7th and 8th integer arguments in `rax`/`rbp` — an internal-only extension of the SysV list — so an affected call received garbage with no diagnostic.)

`RETURN <struct-slot>` builds the `CSTRUCT`'s mapped record from the buffer the callee filled, so the wrapper must return that record type. `RETURN` on an `IN` struct slot is rejected: an input slot is zeroed and never read back. Field values marshal by width and signedness — a signed narrow field is sign-extended (so a `CInt32` of `-1` surfaces as `-1`, not `4294967295`), a `CBool` normalizes to `TRUE`/`FALSE`, and a `CDouble` that is NaN or infinite is rejected, since an MFBASIC `Float` is always finite. Writing a field range-checks the same way a `CInt32` argument does: a 64-bit `Integer` that does not fit the C field fails with `ErrOverflow` rather than truncating. [[src/target/shared/code/link_thunk.rs:marshal_struct_out]]

**A native resource that carries `STATE` (`BIND STATE`).** A native `LINK` function that produces a resource may declare a `STATE T` clause on its `RES` return — `FUNC openFile(path) AS RES SoundFile STATE FileInfo` — exactly as an ordinary function does (§15.5). Such a resource carries STATE the same way a built-in `File STATE T` does: `.state` reads it, a state update made through any pointer to it is visible to the owning scope, and drop reclaims it.

**Every** native resource is an 80-byte resource **record** — a handle at offset 0, a closed flag at offset 8, and a STATE payload pointer at offset 16 — whether or not it declares `STATE`. A stateless native resource is a record whose STATE pointer is null, not a bare `CPtr`; the native symbol still receives the raw handle, which the thunk loads from the record before the call. The uniform representation is what gives a stateless resource a closed flag, which a bare handle had nowhere to store.

`BIND STATE <res-slot> = <out-struct-slot>` populates that STATE from a struct the native call filled through an `OUT` parameter: the thunk marshals the filled `CSTRUCT` (via its `AS S` record, exactly as `RETURN <struct-slot>` does) into the returned resource's STATE payload. This is the shape a library like libsndfile needs — `sf_open` hands back an `SNDFILE*` and fills an `SF_INFO`, which becomes the handle's state:

```basic
FUNC openFile(path AS String) AS RES SoundFile STATE FileInfo
  SYMBOL "sf_open"
  ABI (path CString, mode CInt32, info OUT SfFileInfo) AS file CPtr
  CONST mode = 16
  BIND STATE file = info        ' the returned resource carries the filled SF_INFO
  ERROR_ON file = NOTHING
  RETURN file
END FUNC
```

Without `BIND STATE`, a stateful native return's STATE default-initializes (the caller's binding allocates it) exactly as a built-in resource's does. `<out-struct-slot>` must be an `OUT` `CSTRUCT` slot whose `AS S` record matches the resource's declared STATE type, and `<res-slot>` must name the slot the wrapper returns — the STATE always attaches to the returned resource, so naming any other slot is a mistake and is rejected (`NATIVE_BIND_STATE_INVALID`) rather than silently ignored. A native FUNC may declare at most one `BIND STATE`. A resource's STATE type is fixed: every native declaration that names it — a producer's return, the close op's `RES` parameter — must agree on it (`TYPE_STATE_MISMATCH`), since the payload carries no runtime tag. The stateful return and its STATE ride an exported signature, so an importer binds the resource and reads its `.state` across the package boundary. [[src/target/shared/code/link_thunk.rs:lower_link_thunk]]

**`CString` struct fields.** A `const char *` field marshals both ways. Coming **out**, the pointer is copied into an owned MFBASIC `String`, its bytes validated as UTF-8 (`ErrEncoding` if not), and a NULL yields `""`. Going **in**, a `String` field becomes a NUL-terminated C buffer that lives for the duration of the call.

The out direction is **copy-and-leave**: the wrapper copies the bytes and never frees the source. That is correct when the C library owns the storage and keeps it valid — libsndfile's format names live in a `static const` table, so freeing them would be a wild free. There is deliberately **no per-field `FREE`**: the compiler cannot tell a library-owned pointer from a caller-owned one (it is a fact about the C API, not the type), so a struct field is always copy-and-leave. **Using a `CString` struct field for caller-owned storage therefore leaks it** — a binding that needs to release a returned pointer must take it as the wrapper's own `CPtr` result and use a `FREE` block. A `CPtr` field is rejected outright and always will be: a raw pointer may not surface in a record.

A `String` field carries the marshaler's existing embedded-NUL gap: the copy does not scan for an interior NUL, so a `String` containing one truncates on the C side. [[src/target/shared/code/link_thunk.rs:emit_copy_cstring_to_string]]

ABI parameters may use a direction modifier:

| Form | Meaning |
|------|---------|
| `OUT T` | Pass a pointer to native storage and copy the produced value back after the call. The pointer lifetime ends when the native call returns. |
| `CPtr` | Pass a resource handle or opaque pointer as-is inside the binding boundary. |

> Implementation status: there is **no `REF` direction modifier** in the parser — `abiSlot` accepts only an optional `OUT`. An ordinary (input) slot is marshaled by value from its bound wrapper parameter or `CONST` pin.

**Bulk byte output (`CBuffer` and `BUFFER <slot> SIZE <expr>`).** Every other ABI ctype is fixed-width: its size is a constant the compiler knows, and both the C struct layout computer and the thunk's frame layout rest on that. A C bulk-read API — `sf_readf_short`, `read`, `recv` — instead takes a caller-allocated buffer and a capacity, and the capacity is computed from the *other* arguments. `CBuffer` is the ctype for that slot, and `BUFFER <slot> SIZE <expr>` is where its capacity is stated:

```basic
FUNC readFrames(file AS RES SoundFile, frames AS Integer, channels AS Integer) AS List OF Byte
  SYMBOL "sf_readf_short"
  ABI (sndfile CPtr, buf OUT CBuffer, frames CInt64) AS read CInt64
  BUFFER buf SIZE frames * channels * 2
  RETURN buf LENGTH read * channels * 2
END FUNC
```

`SIZE` is in **bytes**, and the expression may read the wrapper's **parameters** and its **`CONST` pins** — nothing else. Naming anything else is `NATIVE_ABI_UNBOUND_SLOT`.

That is narrower than `SUCCESS_ON` / `RETURN`, and deliberately so. Those are evaluated *after* the native call, so they may read the ABI return and `OUT` slots. A `BUFFER … SIZE` decides how many bytes to allocate *before* the call runs, so the ABI return and every `OUT` slot are still uninitialized at that point. Naming one is not a typo but a causality error, and it would silently size a buffer from stack garbage — so it is rejected rather than accepted and read.

Deriving the capacity from a sibling slot by naming convention (a `buflen CInt64` next to a `buf OUT CBuffer`) was rejected: it is implicit, unstated in the `ABI` line, and silently picks the wrong slot whenever a C function takes two lengths. The clause states the relationship the C API actually has.

A `CBuffer` is legal in exactly one position, and every other use is rejected with `NATIVE_BUFFER_INVALID` on both the source path and the `.mfp` package path:

* It must be `OUT`. `IN`/`INOUT CBuffer` would need a `List OF Byte` *input* marshal, which does not exist; there is no send direction.
* It must carry exactly one `BUFFER … SIZE` clause. Zero leaves the capacity undefined, more than one leaves it ambiguous.
* A `BUFFER` clause must name a `CBuffer` slot of the same function.
* It must be the slot `RETURN` names. Unlike a scalar `OUT`, which merely goes unread, an unreturned buffer costs a runtime-sized allocation whose bytes nothing can observe.
* The wrapper's return type must then be `List OF Byte` — and, conversely, **a wrapper returning `List OF Byte` must return a `CBuffer` slot.** Nothing else can produce a byte list.
* It cannot be a `CSTRUCT` field (a struct field needs a constant offset) and cannot be the ABI return proper (`AS r CBuffer`): a C function fills storage the caller passed in, it does not return a buffer whose size the caller declared.
* It **must** carry a `RETURN <slot> LENGTH <expr>` clause. See below.

**Reporting what was written (`RETURN <slot> LENGTH <expr>`).** The buffer is allocated at its full `SIZE` capacity, but a C bulk read routinely writes less — a short read, an EOF, an error. `LENGTH` says, in **bytes**, how much the callee actually wrote, and it is what the returned list's length becomes:

```basic
  BUFFER buf SIZE frames * channels * 2
  RETURN buf LENGTH read * channels * 2
```

Unlike `SIZE`, a `LENGTH` expression is evaluated **after** the call, so it may read the ABI return and any `OUT` slot — which is the point, since that is where the callee reports what it wrote. The value is **clamped to `[0, capacity]`**: `read(2)`, `pread(2)` and `sf_read_short` all return `-1` on error and `0` at EOF, and an unclamped negative stored as a length is a huge *unsigned* value that would send every later read off the end of the block.

`LENGTH` is **mandatory** on a returned `CBuffer`, not optional. Without it the list's length would be the buffer's full capacity, so a callee that short-writes would leave the remainder as uninitialized arena memory readable as ordinary data. Requiring the clause closes that by construction and costs nothing at run time; zero-filling every buffer instead would mean an O(N) write on every call, including the calls that fill it completely. A callee that always fills the buffer simply writes `LENGTH n`.

The buffer's *capacity* deliberately stays at the full `SIZE` after truncation. That is what lets the arena reclaim the whole block — block size is computed from capacity, so lowering it would leak the tail — and `capacity > count` is sanctioned headroom (§Collections); a value-semantic copy is shrink-to-fit, so the slack disappears the first time the list is copied.

A `SIZE` that is negative, or larger than the project's buffer ceiling, raises `ErrInvalidArgument` **before anything is allocated**: the size comes from a wrapper parameter, so without a ceiling it is an unbounded allocation request driven by the caller.

That ceiling is **64 MiB** by default and is set per project by the `maxBuffer` field in `project.json`, in MiB:

```json
{
  "name": "recorder",
  "maxBuffer": 256
}
```

Valid values are whole numbers from 1 to 4096 (MiB); anything else is `PROJECT_JSON_FIELD_TYPE`. The default of 64 MiB is ~5.8 minutes of stereo 48 kHz 16-bit audio.

`maxBuffer` is the **consuming** project's setting, not the binding's. `LINK` thunks are emitted when an executable links, so the application that imports a binding decides how much memory one native read may claim — a binding cannot raise an application's ceiling on its behalf, and a package does not carry one.

> Implementation status (plan-58-B): `OUT CBuffer` marshals — the thunk allocates the byte list, hands the callee a pointer to its data region, and truncates to `LENGTH` on return. `BUFFER` and `LENGTH` clauses do **not** yet ride the `.mfp` wire format (plan-58-C owns it), so a `CBuffer` binding cannot currently be consumed from a compiled package: decoding one yields a slot with no `BUFFER` clause, which fails to lower with a diagnostic rather than marshalling a zero-capacity buffer.

**Pinning constant and NULL arguments (`CONST slot = value`).** The `ABI (...)` line always states the true native signature — every C argument in C order. Some of those arguments are fixed values the caller never supplies (a `-1` length, a NULL callback, a sentinel destructor). `CONST <slot> = <value>` pins one ABI slot to a fixed value and removes it from the wrapper's parameter list. The value is checked against the slot's declared ABI type. `NOTHING` pins a C NULL on a pointer slot; a pointer-sized integer literal pins a sentinel pointer (e.g. `-1` for SQLite's `SQLITE_TRANSIENT`). A `CONST` slot is input-only — marking it `OUT` or as the result is rejected (`NATIVE_CONST_OUT`), and pinning an unknown slot is `NATIVE_CONST_UNKNOWN_SLOT`. A pin is call metadata baked into the native frame; it never materializes as a source value, so it cannot forge or leak a `CPtr`. A pinned integer is lowered as a 64-bit **bit pattern**, so the full unsigned range is available: `CONST flags = 0xFFFFFFFFFFFFFFFF` pins all sixty-four bits, exactly as the equivalent `-1` does. [[src/ir/lower_link.rs:link_const_bits]]

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

An identifier in `<expr>` names an `ABI (...)` slot or the ABI return, and is resolved against them: a name that is neither is rejected with `NATIVE_ABI_UNBOUND_SLOT`. The ABI return resolves to the **sign-extended** status, so `ERROR_ON status = -1` compares against `-1` and not `4294967295`. An `OUT` slot resolves to its produced value, so a gate may read what the callee wrote. [[src/ir/lower_link.rs:lower_link_expr]] [[src/target/shared/code/link_thunk.rs:emit_link_expr]]

> Implementation status: until plan-50-I, `<expr>` did not actually range over slot names — lowering collapsed *every* identifier onto a single nameless variable meaning "the native return". So `SUCCESS_ON typo = 0` silently meant `SUCCESS_ON status = 0`, and `SUCCESS_ON count = 5` over an `OUT` slot could not mean what it said. It never bit any in-tree binding, because the only name any of them used *was* the return name.

**Naming the result (`RETURN <expr>`).** A wrapper's result is whatever `RETURN <expr>` names. There is exactly one such clause, and `<expr>` is the same expression language `SUCCESS_ON` uses — so `RETURN db` is the degenerate case where the expression is a bare slot reference, and `RETURN status = 100` is the computed case.

`RETURN <slot>` over an `OUT` slot surfaces the value the callee produced (a handle, a count); `RETURN <abi-return-name>` passes the C return through, marshaled by its ctype; and any other expression computes the result from the named slots. For example SQLite's `sqlite3_step` returns `SQLITE_ROW` (100) or `SQLITE_DONE` (101); the wrapper returns `AS Boolean` meaning "a row is ready":

```basic
FUNC step(RES stmt AS Stmt) AS Boolean
  SYMBOL "sqlite3_step"
  ABI (stmt CPtr) AS status CInt32
  SUCCESS_ON status = 100 OR status = 101   ' both are non-errors
  RESULT status = 100                       ' TRUE iff a row is ready
END FUNC
```

A plain value-returning call needs no gate: name the C return and hand it back, e.g. `ABI (stmt CPtr, name CString) AS value CInt32` + `RETURN value`. A value-producing wrapper with no `RETURN` is rejected (`NATIVE_ABI_NO_RESULT`); a `Nothing` wrapper with one is `NATIVE_ABI_RESULT_MARKER`.

> Implementation status: before plan-50-H the result was identified by a **magic slot name**. A slot literally named `return` (or an `AS return <ctype>` native return) *was* the result, and a separate `RESULT <expr>` clause supplied a computed one. That worked only because the compiler forced every `OUT` slot to be named `return`, making "is OUT", "is named `return`", and "is the result" indistinguishable — a coincidence `INOUT` struct slots break, since `sf_open` fills an `INOUT` slot whose value is *not* the result. `RETURN <expr>` replaced both spellings and absorbed `RESULT`, so `return` is a keyword again and not a legal slot name.

**Multiple outputs — not implemented.** A wrapper surfaces exactly one result, so an ABI signature whose C function produces two independent outputs cannot express both. A binding works around it by splitting the call, or by using a follow-up query that returns the second value on its own (libsndfile's `sf_open` fills an `SF_INFO` *and* returns a handle; a binding takes the handle from `sf_open` and reads the info back with `SFC_GET_CURRENT_SF_INFO`).

```basic
TYPE DivModResult
  quotient AS Integer
  remainder AS Integer
END TYPE

LINK "mylib" AS mylib
  FUNC divmod(a AS Integer, b AS Integer) AS DivModResult
    SYMBOL "divmod"
    ABI (a CInt32, b CInt32, quotient OUT CInt32, remainder OUT CInt32) AS CVoid
    ' No way to name both OUT slots as the result — DEFERRED, not compilable.
  END FUNC
END LINK
```

> Implementation status: multiple `OUT` slots parse and marshal (each gets its own buffer), but only one can be named by `RETURN`, so the others are unreachable. An earlier design reserved a `RETURN_OUT DivModResult[quotient, remainder]` clause for this; it was **retired** by plan-50-H, whose `RETURN <expr>` covers the single-output case that `RETURN_OUT` was mostly there to spell. Reviving multi-output means extending `RETURN` to construct a record from several slots, not adding a second clause. [[src/ast/items.rs:parse_link_function]]

**Freeing a caller-owned return (`FREE`).** A `CPtr` result mapped to an owned MFBASIC value (such as `AS String`) is **copied** out of the native buffer and the source pointer is then left untouched — *copy-and-leave*. That is correct when the native library **owns** the buffer and keeps it valid (a transient or static pointer), as with `sqlite3_column_text`. When the call instead returns a buffer the **caller owns and must release** — `sqlite3_expanded_sql`, `sqlite3_mprintf`, `strdup` — copy-and-leave would leak it. A `FREE` block names the produced slot and the deallocator that releases it:

```basic
LINK "sqlite3" AS sqlite
  FUNC expandedSql(RES stmt AS Stmt) AS String
    SYMBOL "sqlite3_expanded_sql"
    ABI (stmt CPtr) AS text CPtr
    FREE sql
      SYMBOL "sqlite3_free"
      ABI (ptr CPtr) AS CVoid
    END FREE
  END FUNC
END LINK
```

`FREE sql` means: after the wrapper has copied the `return` slot into its owned MFBASIC result, pass the **original** native pointer to the named deallocator. The nested `SYMBOL`/`ABI` declare that deallocator — exactly one pointer parameter and a `CVoid` return. The freed slot is the produced pointer: the C `return`, or a named `OUT` slot. The deallocator runs **once, after the copy, on the success path only**; if the wrapper fails before the value is produced (a failed `SUCCESS_ON` gate, a marshaling error), nothing is freed. A NULL produced pointer is passed to the deallocator unchanged, because deallocators such as `sqlite3_free` define NULL as a no-op. The original pointer is never surfaced as an MFBASIC value, so `FREE` is the only sanctioned way to release a caller-owned native return — a raw `CPtr` cannot be handed back to source code to free by hand (`NATIVE_CPTR_ESCAPE`). A binding with more than one caller-owned pointer (for example several `OUT` buffers) states one `FREE` block per slot.

Rules:

- `LINK` names and all declared `SYMBOL` names are resolved before `main` starts. Native libraries are not lazy-loaded. The backend emits a load-time initializer `_mfb_linker_init` that `dlopen`s each distinct library with `RTLD_NOW` and `dlsym`s every declared symbol (and every `FREE` deallocator) into a per-function global pointer slot. [[src/target/shared/code/link_thunk.rs:_mfb_linker_init]]
- **The filename `dlopen` is given is the one the binding author declared** for the build's exact `(os, arch, libc)`, resolved from that binding's `libraries` section (`./mfb spec tooling project-manifest`). The compiler does not synthesize a soname: the old `lib<logical>.so.0`/`lib<logical>.dylib` guess is gone, because it missed every unversioned `.so`, `.so.3`, non-`lib`-prefixed, and per-arch/libc variant. See *Locator resolution* below. [[src/target/shared/code/link_locator.rs:resolve]]
- If a required native library or symbol cannot be loaded before `main`, the initializer returns an error `Result` carrying `ErrNativeBindingUnavailable` (`77030007`, `ERR_NATIVE_LINK_LOAD_CODE`). The program entry handles this exactly like a failed global initializer and aborts before running `main`. (Note: this differs from the build-time linker diagnostic `5-500-0001`/`LINK_FAILED`, not the runtime load-failure code.)
- Linked names occupy a package-like namespace. A package-qualified name such as `sqlite::open` follows the same two-part rule as package access.
- A native call may resolve only the symbols declared by `SYMBOL` entries in the binding package. Dynamic lookup by source strings or computed names is not available to ordinary MFBASIC code.
- Native functions expose ordinary MFBASIC signatures. At call sites they auto-unwrap, auto-propagate, and participate in `MATCH` like any other fallible function.
- Native functions may accept and return MFBASIC primitive values, strings, byte lists, and declared resource types through an explicit `ABI` mapping. Other conversions are implementation-defined unless specified by the binding.
- A value-returning native function must name its result with exactly one `RETURN <expr>`; otherwise it is rejected with `NATIVE_ABI_NO_RESULT`. A `Nothing` wrapper must not declare one (`NATIVE_ABI_RESULT_MARKER`). A slot named `return` does not parse — the name carries no meaning and is a keyword.
- A `FREE` block must name a `CPtr`-typed produced slot — the `return` slot or a declared `OUT` slot — and its deallocator must declare exactly one pointer parameter and a `CVoid` return. The deallocator is called once on the success path, after the produced value is copied into the wrapper's owned MFBASIC result, with the original (possibly NULL) native pointer; it is not called on a failed call. Without a `FREE` block a `CPtr` result is copied and the source pointer is left untouched (copy-and-leave), which leaks a caller-owned buffer — `FREE` is the only way to release one.
- `RESOURCE` is a declaration form for concrete opaque unique-handle types; it is not an inheritance base type and cannot be used as a generic catch-all type.
- Native resource ownership is declared at package scope with `RESOURCE <Name> CLOSE BY <closeFn>`. Raw C ABI types (`CPtr`, `CString`, `CInt32`, …) may appear only inside `ABI (...)` slots, never in a wrapper's MFBASIC-facing signature; a `CPtr` exists solely as the hidden representation of a declared resource and must not escape into an ordinary API (`NATIVE_CPTR_ESCAPE`).
- `OUT` native pointer values are temporary call-frame values (there is no `REF` modifier). Native code must not retain them after return; if a binding needs retained native storage, it must model that storage as a declared `RESOURCE`.
- An `ABI (...)` slot's C type, and the ABI return's, must name a type the marshaling backend implements; anything else is `NATIVE_ABI_UNKNOWN_CTYPE`. `CVoid` is return-only and `CString` argument-only (see the type table above). Enforced on both the source and package paths.
- A `CSTRUCT`'s layout is computed from its field ctypes and is never declared or transported; offsets are recomputed when a package is decoded. A struct the compiler cannot lay out faithfully is rejected (`NATIVE_CSTRUCT_INVALID`), and one over 1024 bytes is `NATIVE_CSTRUCT_TOO_LARGE`.
- A `CSTRUCT` name is nameable only in its declaration, an `ABI (...)` slot's ctype position, and `SIZEOF`; anywhere else is `NATIVE_CSTRUCT_ESCAPE`.
- A struct slot's buffer is always fully zeroed before the call, so an unbound field is `0` and no stack contents reach the C library. `BIND IN` writes its inputs; `RETURN <slot>` reads the result back as the `CSTRUCT`'s mapped record. `INOUT` is valid only on a struct slot.
- Native `LINK` resources slot into the resource model of §15 unchanged: bound with `RES`, passed as a pointer at ordinary calls (which do not move ownership), auto-closed by lexical drop through the registered close op, never copied/stored/field-accessed, and thread-sendable only with `THREAD_SENDABLE`. Diagnostics specific to native bindings use codes `1-102-0008`…`0009`, `2-203-0089`…`0098`, and `2-203-0123` (see `./mfb man errors`).
- Native libraries are platform-specific dependencies. A binding package declares which concrete shared object to load per platform in its project.json `libraries` section; the compiler encodes those locators into the `.mfp` (`./mfb spec package native-bindings`), together with a sha256 for any library the author vendors. The native library itself is never portable binary representation and is not carried inside the `.mfp` — only its name, or its hash.
- Two **built-in** packages also load platform libraries at run time, through the same `dlopen`/`dlsym` mechanism but internal to their runtime helpers rather than via a `LINK` block: `tls` (Network.framework/Security.framework on macOS, `libssl`/`libcrypto` on Linux) and `crypto` (Security.framework SecKey on macOS, `libcrypto.so.3` falling back to `libcrypto.so.1.1` on Linux, for the NIST-EC public-key operations only — every other `crypto` primitive is a portable software core). These built-ins resolve their symbols lazily inside each helper call, not in `_mfb_linker_init`, and surface load failures as ordinary package errors (`ErrTlsFailed` / `ErrUnknown`), not `ErrNativeBindingUnavailable`. See `./mfb spec stdlib crypto` for the crypto backend split.

**Example:**

```

EXPORT RESOURCE Db CLOSE BY sqliteLink::close
RESOURCE Stmt CLOSE BY sqliteLink::finalize

LINK "sqlite3" AS sqliteLink
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, db OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC openV2(path AS String, flags AS Integer) AS RES Db
    SYMBOL "sqlite3_open_v2"
    ABI (path CString, db OUT CPtr, flags CInt32, zVfs CPtr) AS status CInt32
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
    ABI (db CPtr, sql CString, nByte CInt32, db OUT CPtr, pzTail CPtr) AS status CInt32
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
    ABI (stmt CPtr, name CString) AS value CInt32
  END FUNC

  FUNC step(RES stmt AS Stmt) AS Boolean
    SYMBOL "sqlite3_step"
    ABI (stmt CPtr) AS status CInt32
    SUCCESS_ON status = 100 OR status = 101
    RESULT status = 100
  END FUNC

  FUNC columnText(RES stmt AS Stmt, col AS Integer) AS String
    SYMBOL "sqlite3_column_text"
    ABI (stmt CPtr, col CInt32) AS text CPtr
  END FUNC

  FUNC columnType(RES stmt AS Stmt, col AS Integer) AS Integer
    SYMBOL "sqlite3_column_type"
    ABI (stmt CPtr, col CInt32) AS value CInt32
  END FUNC

  FUNC columnInt(RES stmt AS Stmt, col AS Integer) AS Integer
    SYMBOL "sqlite3_column_int64"
    ABI (stmt CPtr, col CInt32) AS value CInt64
  END FUNC

  FUNC columnDouble(RES stmt AS Stmt, col AS Integer) AS Float
    SYMBOL "sqlite3_column_double"
    ABI (stmt CPtr, col CInt32) AS value CDouble
  END FUNC

  FUNC columnCount(RES stmt AS Stmt) AS Integer
    SYMBOL "sqlite3_column_count"
    ABI (stmt CPtr) AS value CInt32
  END FUNC

  FUNC finalize(RES stmt AS Stmt) AS Nothing
    SYMBOL "sqlite3_finalize"
    ABI (stmt CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC expandedSql(RES stmt AS Stmt) AS String
    SYMBOL "sqlite3_expanded_sql"
    ABI (stmt CPtr) AS text CPtr
    FREE sql
      SYMBOL "sqlite3_free"
      ABI (return CPtr) AS CVoid
    END FREE
  END FUNC

  FUNC errmsg(RES db AS Db) AS String
    SYMBOL "sqlite3_errmsg"
    ABI (db CPtr) AS text CPtr
  END FUNC

  FUNC extendedErrcode(RES db AS Db) AS Integer
    SYMBOL "sqlite3_extended_errcode"
    ABI (db CPtr) AS value CInt32
  END FUNC

  FUNC errstr(code AS Integer) AS String
    SYMBOL "sqlite3_errstr"
    ABI (code CInt32) AS text CPtr
  END FUNC
END LINK
```

## Locator resolution

`LINK "sqlite3"` names a **logical** library. Which concrete shared object that
becomes is declared by the binding author's project.json `libraries` section (see
`./mfb spec tooling project-manifest` for the schema) and carried in the `.mfp` as
the `NATIVE_LIBRARY_TABLE`. At **executable** build time the compiler resolves,
for the exact `(os, arch, libc)` being emitted, the **most-specific** matching
locator.

A locator matches when its `os` equals the target's, and its `arch` and `libc` are
each either absent (a wildcard meaning *any*) or equal to the target's. Among
matches, **specificity** is the number of axes pinned — `(arch specified? 1:0) +
(libc specified? 1:0)` — and the highest wins:

| locator | building `linux/riscv64/musl` | specificity |
| --- | --- | --- |
| `{os: linux, type: system, source: libsqlite3.so.0}` | matches (both wildcards) | 0 |
| `{os: linux, arch: riscv64, libc: musl, source: libsqlite3-riscv64-musl.so}` | matches | 2 |

The vendored build wins here; every other Linux slot falls to the system soname.
"Vendor on one slot, the system library everywhere else" therefore needs no
special rule — and because a Linux `vendor` locator must pin both axes, it always
outranks a wildcarding `system` entry for its exact slot.

- No matching locator → `NATIVE_LIBRARY_NO_MATCH` (**build error**, not a runtime
  `ErrNativeBindingUnavailable`). A build error beats emitting a soname that
  cannot load.
- Two equally-specific matches → `NATIVE_LIBRARY_AMBIGUOUS` (build error). Only
  reachable via genuinely duplicate entries.

A Linux `mfb build` emits both libc flavors from one invocation, each its own
codegen pass with its own data image, so a locator that differs per libc lands in
the correct binary automatically.

### What gets `dlopen`ed

The emitted string is always a bare **filename**, never a path — the loader finds
it, via the system search path for a `system` locator or the executable's RPATH
for a `vendor` one:

- a **`system`** locator emits its `source` verbatim: the exact soname;
- a **`vendor`** locator emits `<declaring-unit>-<source>`, the disambiguated name
  the build copies the file under. Vendor filenames are unique only *within* one
  manifest, and the output flattens every vendor file into one directory, so two
  packages each shipping a `libfoo.so` would otherwise silently load one another's
  library.

### Vendor verification

A `.mfp` carries a vendored library's **sha256, never its bytes**. Where the file
comes from depends on who declared the locator:

| whose locator | vendor file read from | who puts it there |
| --- | --- | --- |
| the project's own `libraries` section | `<project>/vendor/<source>` | the author, by hand |
| an imported binding's section-10 table | `<project>/packages/<name>.vendor/<source>` | `mfb pkg add`/`pkg install`, automatically |

An imported binding's vendored libraries **arrive with the package**: publishing
uploads each one to the registry as its own content-addressed blob, and
installing downloads every blob the package's section-10 table names, verifying
each against the signed hash before it is allowed to exist under a usable name.
Nothing has to be placed by hand. (See `./mfb spec package-manager
repository-protocol` for the blob endpoints and the ordering argument.)

At build time the compiler hashes the resolved file and compares it to the
recorded digest, whichever root it came from:

- missing/unreadable → `NATIVE_LIBRARY_FILE_MISSING` (build error, naming the full
  expected path);
- digest differs → `NATIVE_LIBRARY_HASH_MISMATCH` (build error — the wrong version
  of the library).

### How a vendored library is found at run time

The verified file is copied into the build's output directory and the executable
carries an **RPATH** pointing at it, so `dlopen` of the bare filename resolves —
from any working directory, and after the whole output directory is moved:

| build | rpath | vendor files |
| --- | --- | --- |
| linux console | `$ORIGIN/vendor` (`DT_RUNPATH`) | `build/vendor/` |
| linux `--app` | `$ORIGIN/../lib` (`DT_RUNPATH`) | `build/<name>.AppDir/usr/lib/`, and therefore inside the sealed `<name>.AppImage` |
| macos console | `@loader_path/vendor` (`LC_RPATH`) | `build/vendor/` |
| macos `--app` | `@executable_path/../Frameworks` (`LC_RPATH`) | `build/<name>.app/Contents/Frameworks/` |

The two Linux shapes differ by exactly that one string: an app build's executable
sits at `usr/bin/<name>` inside the AppDir, one directory below its libraries, so
it cannot share the console build's `$ORIGIN/vendor`. Because the AppImage is a
*sealed* file, the vendored libraries are copied into the AppDir **before** the
seal closes it (`finalize_app_bundle` runs after `copy_vendor_libraries`); nothing
can be added afterwards.

A build with no `vendor` locators emits **no** RPATH and no vendor directory, and
its bytes are identical to a build predating the feature. `DT_RUNPATH` (tag 29) is
used rather than the deprecated `DT_RPATH`, so `LD_LIBRARY_PATH` can still
override it. RPATH is what keeps `dlopen` a bare-filename call with no runtime
code: the loader resolves it, and the executable's own `DT_RUNPATH`/`LC_RPATH` is
consulted for a `dlopen` issued from the executable itself, which is where
`_mfb_linker_init` lives.

Only *resolved* locators are copied — a project vendoring blobs for six targets
ships one per build. Both Linux libc flavors share the one `build/vendor/`, which
is sound because vendor filenames are unique project-wide.

Each copied file is named `<declaring-unit>-<source>` — the package (or project)
that declared the locator, then the filename — because the output directory is
flat and the filename *is* the library's identity to `dlopen`. Two packages each
vendoring a `libfoo.so` would otherwise silently load one another's library.

The *input* side is disambiguated too, by a different mechanism: an imported
binding's vendored files live in a per-package `packages/<name>.vendor/`
directory, so two imported packages that each vendor a `libfoo.so` hold distinct
files on disk and both resolve. The prefix then keeps them distinct in the flat
output directory. Only a project that shares its own name with one of its
dependencies could still collide, which
`NATIVE_LIBRARY_VENDOR_COLLISION` catches and which should never occur.

The build performs **no signature check** on a vendored dylib, and adds no
diagnostic for it. Worth knowing rather than discovering: Apple Silicon requires
loadable code to carry at least an ad-hoc signature, so an unsigned vendored
`.dylib` may be refused by `dyld` regardless of where it sits. Satisfying that is
the vendoring author's responsibility; most distributed dylibs already are signed.

Automated acquisition remains out of scope: nothing fetches a vendored library
from a registry or embeds it in the `.mfp`.

## See Also

* ./mfb spec tooling project-manifest — the `libraries` locator schema
* ./mfb spec package native-bindings — how `LINK` metadata is carried in `.mfp`
* ./mfb spec linker import-selection — native import resolution at link time
* ./mfb spec language resource-management — the resource model `LINK` handles join
* ./mfb man errors — diagnostics raised for native bindings

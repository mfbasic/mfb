# Native Binding Metadata

A package containing `LINK` declarations is still a normal `.mfp` package. The application imports it normally. The binding metadata lives inside the signed binary representation payload.

The native interface separates MFBASIC-facing wrapper signatures from C-facing ABI signatures, with `CString`/`CPtr`/`OUT`/`SUCCESS_ON`/`ERROR_ON`-style rules. The `.mfp` stores those rules so importers do not repeat the `LINK`.

## There is no `NATIVE_LINK_TABLE` section

Section id `10` (`NATIVE_LINK_TABLE`) is **reserved but unused**. The current compiler does not emit a separate native-link section, and there is no `NativeLibrary`/`NativeSymbol`/`NativeAbi` table in the format. Instead, native `LINK` metadata rides as an **optional append-only trailer inside the `IR` (`MFBR`) payload**, after the `functions` vector (see `binary-representation` §IR payload structure). The trailer is present only when the project has `LINK` functions or re-export aliases, so `LINK`-free packages stay byte-identical to the pre-feature encoding.

The trailer is two vectors:

```text
linkFunctions   vec<IrLinkFunction>
linkAliases     vec<(alias str, target str)>
```

`vec<T>` is a `u32` count followed by that many elements; `str` is a `u32` byte length followed by UTF-8 bytes; `bool` is a single `0`/`1` byte. The consumer decodes these into the merged project so it can rebuild the marshaling thunks and the re-export routing.

## `IrLinkFunction`

Each `IrLinkFunction` is encoded in this exact field order: [[src/ir/binary.rs:encode_link_function]]

```text
alias            str          MFBASIC-facing wrapper name as exported
name             str          internal lowered name
library          str          library/namespace this symbol is linked from
symbol           str          C symbol name to bind
params           vec<(name str, type str)>     MFBASIC-facing parameters
returnType       str          MFBASIC-facing return type
returnResource   bool         whether the return is an owned resource handle
abiSlots         vec<(name str, ctype str, isOut bool)>   C-facing ABI slots
abiReturnName    str          name of the C-facing return slot
abiReturnCtype   str          C type of the return slot
consts           vec<(slot str, value str)>    fixed constant arguments
successOn        optional IrLinkExpr           SUCCESS_ON predicate
result           optional IrLinkExpr           result/ERROR_ON mapping expression
free             optional (slot str, symbol str)  paired free/close for a returned resource
```

`optional X` is a `u8` present-flag (`0`/`1`) followed by the encoding of `X` when present. Constant argument `value`s are serialized as their string form. C types (`ctype`, `abiReturnCtype`) are stored as **strings** (e.g. the source-level C type spelling), not as a numeric ABI-type enum — there is no `CInt8`/`CInt16`/… code table in the format. Likewise the `OUT`/value distinction for each slot is the per-slot `isOut` boolean, not a numeric direction enum.

`free` records the paired deallocation/close symbol and the slot it applies to, so a returned native resource can be released by the generated lexical drop.

## `IrLinkExpr`

`successOn` and `result` are small predicate/mapping expression trees (`encode_link_expr`), one tag byte per node: [[src/ir/binary.rs:encode_link_expr]]

```text
0 = Var                       the call's raw return value
1 = Int(value str)            an integer literal (serialized as a string)
2 = Compare { op str, lhs, rhs }
3 = And(lhs, rhs)
4 = Or(lhs, rhs)
5 = Not(inner)
```

These encode the `SUCCESS_ON`/`ERROR_ON` conditions and the success/error mapping in a portable form so the importer can regenerate the same success-test and result construction the original `LINK` block specified.

## Whitelisting and safety

Native symbols are whitelisted by this trailer: a package cannot perform dynamic native symbol lookup, and only the `symbol`/`library` pairs recorded here can be bound. The marshaling-safety rules these slots encode — `CString` NUL handling, `CPtr` being usable only inside native bindings (never in ordinary exported signatures), and `OUT` storage living only for the call — are source-level rules owned by `./mfb spec language native-libraries`; this trailer is just their on-disk form.

> Note: runtime helper symbols for compiler-owned built-ins (arena allocation, I/O, etc.) are an architecture/runtime concern provided by the native backend and the OS layer — they are **not** part of the `.mfp` package format and are not described by package metadata. They are documented with the code generator and runtime, not here.

## `RESOURCE_TABLE`

Resource types — both the built-in standard resources (`File`, `Socket`, `Listener`) and native `LINK` resources — are recorded in the optional `RESOURCE_TABLE` section (id `11`). A package that has any resource type emits it; a package with none omits it.

```text
resourceCount       u32

repeated resourceCount times:
  resourceType      typeId
  closeFunctionId   u32
  flags             u32
```

Resource flags:

```text
bit 0 = native resource
bit 1 = standard resource
bit 2 = sendable to thread
bit 3 = close may fail
```

`closeFunctionId` is interpreted by flags: for a **native** `LINK` resource (`native` set, `standard` clear) it is the **string id** of the close op's name; for a built-in **standard** resource (`native` and `standard` both set) it is a sentinel function id (e.g. the built-in fs/net close ids). The native-vs-standard distinction is drawn when the resource exports are built, not in the raw table decode. [[src/binary_repr/builder.rs:package_resource_exports]] [[src/binary_repr/builder.rs:resolve_resource_close_name]]

The current compiler's flag assignment:

* Standard built-in resources (`File`, `Socket`, `Listener`) get `native | standard | close-may-fail`, plus `sendable` when the registry marks the type sendable (`standard_resource_flags`). [[src/binary_repr/writer.rs:standard_resource_flags]]
* A native `LINK` resource gets `native`, plus `sendable`/`close-may-fail` as declared by the `LINK` block (`add_native`). [[src/binary_repr/sections.rs:add_native]]

Default rule: resources are not sendable to threads unless explicitly marked sendable.

## See Also

* ./mfb spec language native-libraries — the source-level `LINK` syntax and marshaling rules
* ./mfb spec linker import-selection — how the bound `(library, symbol)` pairs resolve at link time
* ./mfb spec package binary-representation — the `IR` payload and trailer that carry this metadata

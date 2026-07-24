# Native Binding Metadata

A package containing `LINK` declarations is still a normal `.mfp` package. The application imports it normally. The binding metadata lives inside the signed binary representation payload.

The native interface separates MFBASIC-facing wrapper signatures from C-facing ABI signatures, with `CString`/`CPtr`/`OUT`/`SUCCESS_ON`/`ERROR_ON`-style rules. The `.mfp` stores those rules so importers do not repeat the `LINK`.

## Two structures, two jobs

Native binding metadata is carried in **two separate places**, and the split is deliberate:

- The **interface** — what each wrapper's signature and C ABI are — rides as an optional append-only trailer inside the `IR` (`MFBR`) payload (below). It is valid for any physical file, on any platform.
- The **locators** — *which* concrete shared object to load for a given `os`/`arch`/`libc` — live in the `NATIVE_LIBRARY_TABLE`, section id `10` (see below). These are per *library*, not per *function*, so putting them in the trailer would duplicate the same locator across every symbol.

## `NATIVE_LIBRARY_TABLE` (section id 10)

Emitted **only** for a binding package that declares a `LINK` block; the container's optional flag **bit 0** ("contains native LINK metadata") is set alongside it (see `container-format`). A package with no `LINK` block emits no section 10, leaves bit 0 clear, and is byte-identical to a pre-feature build.

The table is built from the project.json `libraries` section (see `./mfb spec tooling project-manifest`) crossed with the distinct `LINK "<name>"` logical names in the project's IR. It carries **only** linked names: a `libraries` entry with no matching `LINK` warns (`NATIVE_LIBRARY_UNUSED`) and is not encoded, so the section never carries a locator nothing can reach.

```text
NATIVE_LIBRARY_TABLE (section id 10):
  u32 libraryCount
  repeat libraryCount (sorted by logicalName):
    stringId logicalName            // "sqlite3"
    u32      locatorCount
    repeat locatorCount (manifest order):
      stringId os                   // "macos" | "linux"
      stringId arch                 // "" = any arch, else "aarch64"|"x86_64"|"riscv64"
      u8       libc                 // 0 = unspecified (any), 1 = glibc, 2 = musl
      u8       type                 // 0 = system, 1 = vendor
      stringId source               // bare filename: "libsqlite3.dylib"
      // present iff type == vendor (1):
      [32 bytes] hash               // sha256 of <project root>/vendor/<source>
```

Strings are `stringId` into the package's `STRING_POOL`. Entries are sorted by logical name and locators keep manifest order, so the encoding is deterministic.

`source` is a **bare filename** on the wire exactly as in the manifest — the `vendor/` prefix is never encoded. It is a fixed, known location both sides derive; storing it would be redundant data that could disagree with the rule.

A `vendor` locator carries a sha256 of its file, computed at build time by streaming `<project root>/vendor/<source>`; a file that is missing or unreadable is a hard error (`NATIVE_LIBRARY_SOURCE_UNREADABLE`). A `system` locator names a file the producer never sees, so it carries no hash.

### Decode re-validates everything

A `.mfp` is an **untrusted input** on the consumer side, and a locator's `source` feeds both a `dlopen` C string and a `vendor/` path join. The decoder therefore re-checks every invariant the producer was supposed to uphold rather than trusting it: `libc`/`type` in range, `hash` present **iff** `type == vendor`, string ids within the pool, and `source` still a bare filename (no path separator, no `.`/`..`, no drive prefix, no interior NUL). Any violation is a structural decode error. [[src/binary_repr/sections.rs:read_native_library_table]] [[src/manifest/libraries.rs:source_is_bare]]

### Diagnostics

| code | name | severity | trigger |
| --- | --- | --- | --- |
| `2-203-0114` | `NATIVE_LIBRARY_MISSING` | error | a `LINK "name"` has no `libraries` entry |
| `2-203-0115` | `NATIVE_LIBRARY_TARGET_UNCOVERED` | warn | a supported target has no locator (one per uncovered slot) |
| `2-203-0116` | `NATIVE_LIBRARY_SOURCE_UNREADABLE` | error | a `vendor` locator's file is missing or unreadable |
| `2-203-0117` | `NATIVE_LIBRARY_UNUSED` | warn | a `libraries` entry has no matching `LINK` |

The coverage check tests each library's locators against every `(os, arch, libc)` the compiler supports — the backend registry crossed with the libc axis (Linux only), currently **8 slots** (macos-aarch64, three Linux arches × 2 libc, and windows-x86_64). It is derived from the registry, not hardcoded, so registering a backend widens the matrix automatically. Because `arch: None` and `libc: None` are symmetric wildcards, one `{ "os": "linux", "type": "system", "source": "…" }` entry covers all six Linux slots. [[src/manifest/libraries.rs:supported_target_slots]] [[src/manifest/libraries.rs:build_native_library_table]]

## The interface trailer

Native `LINK` interface metadata rides as an **optional append-only trailer inside the `IR` (`MFBR`) payload**, after the `functions` vector (see `binary-representation` §IR payload structure). The trailer is present only when the project has `LINK` functions or re-export aliases, so `LINK`-free packages stay byte-identical to the pre-feature encoding.

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
abiSlots         vec<(name str, ctype str, direction u8)>  C-facing ABI slots
abiReturnName    str          name of the C-facing return slot
abiReturnCtype   str          C type of the return slot
buffers          vec<(slot str, size IrLinkExpr)>   BUFFER <slot> SIZE <expr> clauses
resultLength     optional IrLinkExpr           RETURN … LENGTH <expr>
bindIn           vec<(slot str, fields vec<…>)>     BIND IN field bindings
consts           vec<(slot str, value str)>    fixed constant arguments
successOn        optional IrLinkExpr           SUCCESS_ON predicate
result           optional IrLinkExpr           result/ERROR_ON mapping expression
free             optional (slot str, symbol str)  paired free/close for a returned resource
```

`optional X` is a `u8` present-flag (`0`/`1`) followed by the encoding of `X` when present. Constant argument `value`s are serialized as their string form. C types (`ctype`, `abiReturnCtype`) are stored as **strings** (e.g. the source-level C type spelling), not as a numeric ABI-type enum — there is no `CInt8`/`CInt16`/… code table in the format.

`direction` is a `u8`: `0` = in, `1` = out, `2` = inout. A value outside `0..=2` is a decode **error**, never a silent default — it decides whether the callee writes through the slot. (This field was a bool before `INOUT` existed; a bool could not express three states and two bools would have admitted an illegal fourth.)

`buffers` records the `BUFFER <slot> SIZE <expr>` clauses that size an `OUT CBuffer` slot, and `resultLength` the `RETURN … LENGTH <expr>` that says how many of its bytes the callee wrote. Both are bounded on decode: at most **16** buffers per function, which is already unreachable for real code since a wrapper cannot have more buffer slots than the target has external integer argument registers (6 on x86-64 SysV, 8 elsewhere). The cap exists to bound allocation on a crafted file.

The allocation **ceiling** for a `CBuffer` is deliberately *not* in this record. It is the consuming project's `maxBuffer` (./mfb spec tooling project-manifest), because `LINK` thunks are emitted when an executable links — so a package cannot raise an application's memory ceiling on its behalf.

`free` records the paired deallocation/close symbol and the slot it applies to, so a returned native resource can be released by the generated lexical drop.

## `IrLinkExpr`

`successOn`, `result`, `resultLength` and each buffer's `size` are small predicate/mapping expression trees (`encode_link_expr`), one tag byte per node: [[src/ir/binary.rs:encode_link_expr]]

```text
0 = Var(name str)             the value of a named ABI slot, or of the ABI return
1 = Int(value str)            an integer literal (serialized as a string)
2 = Compare { op str, lhs, rhs }
3 = And(lhs, rhs)
4 = Or(lhs, rhs)
5 = Not(inner)
6 = Mul(lhs, rhs)             integer arithmetic, for BUFFER … SIZE / … LENGTH
7 = Add(lhs, rhs)
8 = Sub(lhs, rhs)
```

`Var` carries the name it reads. It was a *nameless* variant meaning "the native return" until plan-50-I, and lowering mapped every identifier onto it — so `SUCCESS_ON typo = 0` silently meant `status = 0`.

An unknown tag is a decode **error**, never a defaulted node, which is what lets tags be appended without a compatibility window.

Recursion is bounded by the reader's shared decode-depth cap (256 levels), the same guard the op and value decoders use — a crafted package cannot blow the decoder stack through deep nesting.

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

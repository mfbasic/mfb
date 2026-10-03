# Type Table

The `TYPE_TABLE` defines all types referenced by the package binary representation.

```text
typeCount       u32
TypeEntry[typeCount]
```

## Built-in type IDs

These IDs are reserved and do not need table entries: [[src/binary_repr/reader.rs:primitive_type_name]] [[src/binary_repr/sections.rs:type_id]]

```text
1          = Nothing
2          = Boolean
3          = Integer
4          = Float
5          = Fixed
6          = String
7          = Byte
8          = Error
9          = Money
10         = Scalar
11         = AttributedString
12..19     = reserved (future primitives)
0xFFFFFF00 = File      (handle/resource type)
0xFFFFFEFF = Socket    (handle/resource type)
0xFFFFFEFE = Listener  (handle/resource type)
0xFFFFFEFD = TermColor (RETIRED -- see below; never reused)
0xFFFFFEFC = TermSize  (builtin record)
```

Id `0` is unmapped. `Money` uses id `9`, `Scalar` id `10`, and `AttributedString` id `11`. IDs `12..19` are unmapped; table entries begin at `FIRST_TABLE_TYPE_ID` (`20`). Built-in handle names decode as `fs.File`, `tcp.Socket`, and `tcp.Listener` respectively. [[src/binary_repr/mod.rs:TYPE_ATTRIBUTED_STRING]] [[src/binary_repr/reader.rs:primitive_type_name]]

`Error` has reserved primitive identity (the encoder interns field names `code`, `message`), and `TermSize` is a structural builtin record (`columns`/`rows`); referencing it interns those field-name strings but still resolves to the reserved id.

`0xFFFFFEFD` is reserved and never emitted. The reader maps it to `TermColor`; it must not be reused for another type. The current `color::Color` record uses an ordinary table entry. [[src/binary_repr/reader.rs:primitive_type_name]] [[src/binary_repr/sections.rs:type_id]]

All user, package, and instantiated template types appear in the `TYPE_TABLE`. Type table entry ids start at `FIRST_TABLE_TYPE_ID` (`20`), immediately after the low reserved built-in ids (primitives `1..11` plus the reserved band `12..19`), so entry index `0` has type id `20`.

## Type entry

Each entry is 20 bytes followed by its payload in a payload region after the entry array:

```text
kind            u16
flags           u16
name            stringId
ownerPackage    stringId
payloadOffset   u32      (relative to the start of TYPE_TABLE)
payloadLength   u32
```

`flags` is currently **always `0`** — exportedness of a type is not carried here. (The in-memory `abi_export_kind` that marks a type as exported is consumed only to build `ABI_INDEX`; it is not serialized into the type entry. It is set for a source type declared `EXPORT` in the package itself, never for a builtin package's type that rides in the package's IR because its source imports that builtin — the type table may hold `io.Key`, but the package does not export it. [[src/binary_repr/sections.rs:reserve_source_type]]) `ownerPackage` is the interned package/namespace string (empty for compiler-owned templates, `"thread"` for `Thread`/`ThreadWorker`, the package name for user types).

Type kinds:

```text
1  = record               (also how native and standard resource types are encoded)
2  = union
3  = enum
4  = List OF T
5  = Map OF K TO V
6  = Result OF T
7  = Thread OF Msg TO Out
8  = function type
9  = MapEntry OF K TO V
10 = ThreadWorker OF Msg TO Out
11 = <Resource> STATE <T>
12 = foreign type reference
13 = Set OF T
```

There is no distinct "native resource" or "standard resource" kind. `fs.File`, `tcp.Socket`, and `tcp.Listener` use the reserved IDs above. Other resource types, including native `LINK` resources, use **record** entries (kind `1`); its resource-ness — which blocks copying, construction, and field access — is recorded in `RESOURCE_TABLE`, not in the type kind.

There are no open template declarations in package binary representation. `List`, `Set`, `Map`, `MapEntry`, `Result`, `Thread`, and `ThreadWorker` are compiler-owned templates, user templates are expanded by the source compiler, and the type table stores only concrete instantiations such as `List OF Integer`, `Result OF Vec3`, `ThreadWorker OF String TO Integer`, or a user-defined `Stack OF String`. Each distinct instantiation is interned once (keyed by a canonical `Name#id...` string) and reused.

## Record payload

```text
fieldCount      u32

repeated fieldCount times:
  fieldName     stringId
  fieldType     typeId
  visibility    u32     ; 0 = default, 1 = private, 2 = package, 3 = export
```

## Union payload

MFBASIC unions are **tagged unions of named variants**, where each variant carries its own fields — not a structural union of member types. The payload is therefore variant-shaped:

```text
variantCount    u32

repeated variantCount times:
  variantName   stringId
  fieldCount    u32
  repeated fieldCount times:
    fieldName   stringId
    fieldType   typeId
```

Included variants from `UNION ... INCLUDES ...` are flattened into the resulting concrete union (`concrete_union_variants` recursively expands each included union's variants in declaration order, then appends this union's own variants). [[src/binary_repr/writer.rs:concrete_union_variants]] There is no subtype relation; the included union's variants simply become variants of this one.

## Enum payload

```text
memberCount     u32

repeated memberCount times:
  memberName    stringId
  ordinal       u32
```

## `List OF T` payload

```text
elementType     typeId
```

## `Set OF T` payload (kind 13)

A single `elementType` (`u32` type ID), interned as `Set#<elementTypeId>` and decoded as `Set OF <element>`. [[src/binary_repr/sections.rs:set_type]] [[src/binary_repr/reader.rs:decode_type_name_body]]

## Foreign type reference payload (kind 12)

```text
exportKind     u16       (3 = record, 4 = union, 5 = enum)
ownerAbiHash   byte[32]
```

The entry name identifies the foreign type and `ownerPackage` its declaring dependency. It carries the owning type ABI hash instead of duplicating its fields. [[src/binary_repr/sections.rs:foreign_type]]

## `Map OF K TO V` payload

```text
keyType         typeId
valueType       typeId
```

## `MapEntry OF K TO V` payload (kind 9)

A `MapEntry` instantiation (the key/value pair yielded when iterating a `Map`) has the same payload shape as `Map`:

```text
keyType         typeId
valueType       typeId
```

## `<Resource> STATE <T>` payload (kind 11)

```text
baseType        typeId          ' the resource type (kind 1)
stateType       typeId          ' the STATE payload record
```

A resource carrying a `STATE` payload (`./mfb spec language resource-management` §15.5) is a composite of two type ids, interned as `State#<baseTypeId>#<stateTypeId>` and decoded back to the source spelling `<base> STATE <state>`.

The STATE is **not** an ABI distinction — a `File STATE Cursor` and a `File` are the same resource record, the payload being a pointer inside it — but it is a *signature* distinction, so it must round-trip: a consumer reads an imported function's signature from the ABI exports, so a return type encoded without its STATE would compile the exporter and silently degrade every importer to a bare handle. [[src/binary_repr/sections.rs:state_type]]

## `Result OF T` payload

```text
successType     typeId
```

The error member type is always built-in `Error`. The success member `Ok OF T` is compiler-owned and is not emitted as a user-constructible open declaration.

## `Thread OF Msg TO Out` payload

```text
messageType     typeId
outputType      typeId
resourceType    typeId    (present only when the thread carries a resource plane)
```

The trailing `resourceType` is appended only when the thread instantiation has a resource plane. A data-only thread emits just `messageType` and `outputType`, producing an 8-byte payload; a resource-carrying thread produces 12 bytes. The reader checks for the resource ID when payload length is at least 12 bytes. [[src/binary_repr/sections.rs:thread_type]] [[src/binary_repr/reader.rs:decode_type_name_body]]

## `ThreadWorker OF Msg TO Out` payload

Identical shape to `Thread`:

```text
messageType     typeId
outputType      typeId
resourceType    typeId    (present only when the worker carries a resource plane)
```

## Function type payload

```text
flags           u32
paramCount      u32
returnType      typeId

repeated paramCount times:
  paramType     typeId
```

Function type flags:

```text
bit 0 = isolated
```

The current compiler writes `1` when the function type is `ISOLATED`, otherwise `0`; no other bits are produced. (The payload is empty for a function-type entry that could not be parsed back from its canonical name.)

## See Also

* ./mfb spec package metadata-encoding — the string-pool and index conventions the table uses
* ./mfb spec architecture type-name-encoding — the canonical type spelling behind these entries
* ./mfb spec language types — the source-level types these IDs represent

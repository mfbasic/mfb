# Type Table

The `TYPE_TABLE` defines all types referenced by the package binary representation.

```text
typeCount       u32
TypeEntry[typeCount]
```

## Built-in type IDs

These IDs are reserved and do not need table entries (`primitive_type_name` / `TypeTable::type_id` in `src/binary_repr.rs`):

```text
1          = Nothing
2          = Boolean
3          = Integer
4          = Float
5          = Fixed
6          = String
7          = Byte
8          = Error
0xFFFFFF00 = File      (handle/resource type)
0xFFFFFEFF = Socket    (handle/resource type)
0xFFFFFEFE = Listener  (handle/resource type)
0xFFFFFEFD = TermColor (builtin record)
0xFFFFFEFC = TermSize  (builtin record)
```

Id `0` is unused (there is no `Invalid` sentinel constant). Id `9` is **retired** — it was the old `TerminalSize` and is now free. The built-in handle/record types deliberately occupy a high reserved range descending from `0xFFFFFF00` rather than the low range: any id at or above `FIRST_TABLE_TYPE_ID` (`10`) would collide with a per-package table type id and silently corrupt another package's first table type in the signature hash.

`Error` is structural (fields `code`, `message`), and `TermColor`/`TermSize` are structural builtin records (`TermColor` has `r`/`g`/`b`; `TermSize` has `columns`/`rows`); referencing them interns those field-name strings but still resolves to the reserved id.

All user, package, and instantiated template types appear in the `TYPE_TABLE`. Type table entry ids start at `FIRST_TABLE_TYPE_ID` (`10`), immediately after the low reserved built-in ids, so entry index `0` has type id `10`.

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

`flags` is currently **always `0`** — exportedness of a type is not carried here. (The in-memory `abi_export_kind` that marks a type as exported is consumed only to build `ABI_INDEX`; it is not serialized into the type entry.) `ownerPackage` is the interned package/namespace string (empty for compiler-owned templates, `"thread"` for `Thread`/`ThreadWorker`, the package name for user types).

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
```

There is no distinct "native resource" or "standard resource" kind. A resource type (`File`, `Socket`, `Listener`, or a native `LINK` resource) is encoded as an ordinary **record** (kind `1`); its resource-ness — which blocks copying, construction, and field access — is recorded in `RESOURCE_TABLE`, not in the type kind.

There are no open template declarations in package binary representation. `List`, `Map`, `MapEntry`, `Result`, `Thread`, and `ThreadWorker` are compiler-owned templates, user templates are expanded by the source compiler, and the type table stores only concrete instantiations such as `List OF Integer`, `Result OF Vec3`, `ThreadWorker OF String TO Integer`, or a user-defined `Stack OF String`. Each distinct instantiation is interned once (keyed by a canonical `Name#id...` string) and reused.

## Record payload

```text
fieldCount      u32

repeated fieldCount times:
  fieldName     stringId
  fieldType     typeId
  flags         u32
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

Included variants from `UNION ... INCLUDES ...` are flattened into the resulting concrete union (`concrete_union_variants` recursively expands each included union's variants in declaration order, then appends this union's own variants). There is no subtype relation; the included union's variants simply become variants of this one.

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

The trailing `resourceType` is appended only when the thread instantiation has a resource plane. A data-only thread emits just `messageType` and `outputType`, keeping such packages byte-compatible with the pre-resource-plane encoding. A reader distinguishes the two by payload length (8 vs 12 bytes).

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

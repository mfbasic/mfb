# Type Table

The `TYPE_TABLE` defines all types referenced by the package binary representation.

```text
typeCount       u32
TypeEntry[typeCount]
```

## Built-in type IDs

These IDs are reserved and do not need table entries:

```text
0  = Invalid
1  = Nothing
2  = Boolean
3  = Integer
4  = Float
5  = Fixed
6  = String
7  = Byte
8  = Error
9  = TerminalSize
0xFFFFFF00 = File
```

All user, package, and instantiated template types appear in the `TYPE_TABLE`. Type table entry ids start at `10`, immediately after the reserved built-in ids above, so entry index `0` has type id `10`.

## Type entry

```text
kind            u16
flags           u16
name            stringId
ownerPackage    stringId
payloadOffset   u32
payloadLength   u32
```

Type kinds:

```text
1  = record
2  = union
3  = enum
4  = List OF T
5  = Map OF K TO V
6  = Result OF T
7  = Thread OF Msg TO Out
8  = function type
9  = native resource
10 = ThreadWorker OF Msg TO Out
11 = standard resource
```

There are no open template declarations in package binary representation. `List`, `Map`, `Result`, `Thread`, and `ThreadWorker` are compiler-owned templates, user templates are expanded by the source compiler, and the type table stores only concrete instantiations such as `List OF Integer`, `Result OF Vec3`, `ThreadWorker OF String TO Integer`, or a user-defined `Stack OF String`.

## Record payload

```text
fieldCount      u32

repeated fieldCount times:
  fieldName     stringId
  fieldType     typeId
  flags         u32
```

## Union payload

```text
memberCount     u32

repeated memberCount times:
  memberType    typeId
```

Included members from `UNION ... INCLUDES ...` are stored as members of the resulting concrete union. There is no subtype relation.

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

The verifier must reject a `Map` whose key type is not comparable.

## `Result OF T` payload

```text
successType     typeId
```

The error member type is always built-in `Error`. The success member `Ok OF T` is compiler-owned and is not emitted as a user-constructible open declaration.

## `Thread OF Msg TO Out` payload

```text
messageType     typeId
outputType      typeId
```

## `ThreadWorker OF Msg TO Out` payload

```text
messageType     typeId
outputType      typeId
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
bit 1 = sub-compatible Nothing return
```

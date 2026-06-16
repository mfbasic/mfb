# `.mfp` Package Format

A `.mfp` file is a signed MFBASIC package. It contains:

```text
MFP container header
MFB architecture-independent package bytecode
```

The container header provides quick package identity and signature information. The bytecode payload contains the package manifest, dependency metadata, public API metadata, type tables, constants, functions, native binding declarations, and architecture-independent register bytecode.

All integers in `.mfp` files are little-endian. All strings are UTF-8 byte strings and are length-prefixed. No field is NUL-terminated.

## Container layout

```text
.mfp file
  MFPHeader
  packageBytecode
```

## `MFPHeader`

```text
magic              8 bytes
containerMajor     u16
containerMinor     u16
bytecodeMajor      u16
bytecodeMinor      u16
flags              u32

signatureType      u16
signatureLength    u32
signature          byte[signatureLength]

nameLength         u32
name               byte[nameLength]

identLength        u32
ident              byte[identLength]

versionLength      u32
version            byte[versionLength]

identKeyLength     u32
identKey           byte[identKeyLength]

identFingerprintLength u32
identFingerprint       byte[identFingerprintLength]

signingFingerprintLength u32
signingFingerprint       byte[signingFingerprintLength]

authorLength       u32
author             byte[authorLength]

urlLength          u32
url                byte[urlLength]

bytecodeLength     u64

packageBytecode    byte[bytecodeLength]
```

Recommended magic:

```text
4D 46 50 0D 0A 1A 0A 00
M  F  P \r \n SUB \n NUL
```

The magic is deliberately not plain `"MFP1"` so corrupted text-mode transfers are easier to detect.

## Header fields

| Field             | Meaning                                                           |
| ----------------- | ----------------------------------------------------------------- |
| `magic`           | File identification bytes.                                        |
| `containerMajor`  | Major version of the `.mfp` container format.                     |
| `containerMinor`  | Minor version of the `.mfp` container format.                     |
| `bytecodeMajor`   | Required major version of the package bytecode format.            |
| `bytecodeMinor`   | Required minor version of the package bytecode format.            |
| `flags`           | Container-level flags. Unknown required flags reject the package. |
| `signatureType`   | Signature algorithm identifier.                                   |
| `signatureLength` | Number of bytes in `signature`.                                   |
| `signature`       | Package signature bytes.                                          |
| `name`            | Source import name, such as `"sqlite"` or `"geometry"`.           |
| `ident`           | Registry identity `<owner>#<package>` for resolved packages.      |
| `version`         | Package version string.                                           |
| `identKey`        | Owner ident public key for this package ident. |
| `identFingerprint` | Fingerprint of `identKey`. |
| `signingFingerprint` | Fingerprint of the package signing key that verifies `signature`. |
| `author`          | Informational author string.                                      |
| `url`             | Informational package/project URL.                                |
| `bytecodeLength`  | Exact byte length of `packageBytecode`.                           |
| `packageBytecode` | Architecture-independent MFB bytecode image.                      |

The header `name`, `ident`, `version`, `identKey`, `identFingerprint`, `signingFingerprint`, `author`, and `url` are for fast package scanning. The bytecode payload must also contain a signed manifest with the same package identity, owner ident key, owner ident fingerprint, and signing fingerprint. A verifier must reject the package if the header identity and bytecode manifest identity do not match.

## Signature coverage

The package content hash and package signature use the same byte representation:
the entire `.mfp` file with only the `signature` byte range replaced by zero
bytes of the same length.

More precisely:

```text
signatureStart = 26
signatureEnd   = signatureStart + signatureLength

coveredBytes = file[0 : signatureStart]
             || zero[signatureLength]
             || file[signatureEnd : end]

contentHash = SHA-256(coveredBytes)
```

The signature input for `signatureType = 1` is:

```text
"MFP-PACKAGE-v1" || contentHash || ident || version
```

`ident` and `version` in the signature input are the raw header field byte
strings without their length prefixes. The domain string is ASCII and prevents a
package signature from being replayed as another Ed25519 signature type.

The covered bytes include:

```text
magic
containerMajor
containerMinor
bytecodeMajor
bytecodeMinor
flags
signatureType
signatureLength
zero[signatureLength]
nameLength
name
identLength
ident
versionLength
version
identKeyLength
identKey
identFingerprintLength
identFingerprint
signingFingerprintLength
signingFingerprint
authorLength
author
urlLength
url
bytecodeLength
packageBytecode
```

The covered bytes exclude only the actual signature bytes:

```text
signature
```

This signs the package import name, registry ident, owner ident key, owner ident
fingerprint, signing fingerprint, version, container format versions, bytecode
format versions, flags, metadata, and bytecode. `bytecodeLength` is covered, so
truncation, extension, or bytecode replacement invalidates the signature.

Verification must use the raw byte sequence exactly as stored. There is no string normalization, metadata canonicalization, JSON normalization, or re-serialization before verification.

## Signature types

```text
0 = unsigned
1 = Ed25519
```

Rules:

* `signatureType = 0` means the package is unsigned.
* If `signatureType = 0`, then `signatureLength` must be `0`.
* `signatureType = 1` means Ed25519.
* If `signatureType = 1`, then `signatureLength` must be `64`.
* Unknown `signatureType` values reject the package.
* Public registry packages must be signed. `registry:mfb` rejects packages with `signatureType = 0`.
* `mfb pkg install` rejects unsigned packages by default. The only default-permitted exception is a `path:` or `file:` source when the project policy explicitly enables `allowUnsignedLocal`.
* `mfb.lock` must record any unsigned-local exception, including the source package, policy name, and reason.
* A build policy may require a specific `identKey`, `identFingerprint`, `signingFingerprint`, or signing public key for a package ident, package URL, package registry, or package source.

The `identKey`, `identFingerprint`, and `signingFingerprint` are not trusted merely because they appear in the package. Package trust comes from the package manager, registry, local trust store, project lockfile, or explicit user configuration. Registry publish policy must verify that `identFingerprint` is the fingerprint of `identKey`, that the ident key controls the owner in `ident`, and that `signingFingerprint` belongs to the current signing key for that owner.

## Container flags

```text
bit 0 = package contains native LINK metadata
bit 1 = package contains debug metadata
bit 2 = package contains source-map metadata
bit 3 = package is pre-release
bits 4-15 = reserved optional flags
bits 16-31 = reserved required flags
```

If an implementation sees an unknown required flag, it must reject the package before import or merge.

Current compiler source of truth:

- Package/container rejection currently comes from detailed package-reader diagnostics in `src/bytecode.rs`, `src/target/package_mfp/mod.rs`, and `src/main.rs`.
- These failures are currently surfaced as descriptive `error: ...` strings such as invalid magic, invalid signature header, truncated signature, or unsupported bytecode/container version rather than through a single package rule code path.

## Container validation

A reader must reject an `.mfp` package when:

* `magic` does not match. The current compiler reports this as `package does not have the MFP package magic`.
* `containerMajor` is unsupported. The current compiler reports this as `unsupported MFP container major version <n>`.
* `bytecodeMajor` is unsupported. The current compiler reports this as `unsupported MFBC major version <n>`.
* `signatureType` is unknown. The current compiler reports this as `unsupported .mfp signature type <n>`.
* `signatureLength` is invalid for the signature type. The current compiler reports either `unsigned .mfp package must have zero signature length` or `Ed25519 .mfp package must have a 64 byte signature`.
* The signature fails verification under the selected trust policy.
* Any string length exceeds the implementation limit.
* `bytecodeLength` does not exactly match the remaining byte count. The current compiler reports this as `invalid .mfp bytecode length`.
* There are trailing bytes after `packageBytecode`.
* The container header identity does not match the embedded bytecode manifest identity. The current compiler reports this as `MFP header identity does not match bytecode manifest identity`.
* The bytecode manifest package name, ident, version, identKey, identFingerprint, or signingFingerprint do not match the header name, ident, version, identKey, identFingerprint, or signingFingerprint.

Recommended limits:

```text
nameLength                <= 255
identLength               <= 255
versionLength             <= 64
identKeyLength            <= 255
identFingerprintLength    <= 255
signingFingerprintLength  <= 255
authorLength              <= 512
urlLength                 <= 2048
bytecodeLength            <= implementation-defined maximum
```

Package names should use the same identifier restrictions as source package names unless the package manager later defines a wider registry naming scheme.

---

# MFB Package Bytecode

The package bytecode is the architecture-independent payload stored after the `.mfp` header.

The bytecode is not machine code. It contains no native addresses, host pointers, host object layouts, CPU instructions, or platform-specific calling conventions. It is a typed register bytecode plus metadata.

The package bytecode format is called **MFBBC**: MFB Bytecode.

```text
packageBytecode
  BytecodeHeader
  SectionTable
  SectionData...
```

## Bytecode header

```text
bcMagic        4 bytes
bcMajor        u16
bcMinor        u16
bcFlags        u32
sectionCount   u32
sectionTable   SectionHeader[sectionCount]
sectionData    byte[]
```

Recommended `bcMagic`:

```text
4D 46 42 43
M  F  B  C
```

## Section header

```text
sectionId      u16
sectionFlags   u16
reserved       u32
offset         u64
length         u64
```

`offset` is relative to the start of `packageBytecode`, not the start of the file.

Sections may appear in any order, but section ranges must not overlap. Required sections must be present exactly once.

## Section IDs

```text
1  = MANIFEST
2  = STRING_POOL
3  = TYPE_TABLE
4  = CONST_POOL
5  = IMPORT_TABLE
6  = EXPORT_TABLE
7  = GLOBAL_TABLE
8  = FUNCTION_TABLE
9  = CODE
10 = NATIVE_LINK_TABLE
11 = RESOURCE_TABLE
12 = DEBUG_INFO
13 = SOURCE_MAP
14 = AUDIT_INFO
15 = ABI_INDEX
```

Required sections:

```text
MANIFEST
STRING_POOL
TYPE_TABLE
CONST_POOL
IMPORT_TABLE
EXPORT_TABLE
GLOBAL_TABLE
FUNCTION_TABLE
CODE
ABI_INDEX
```

Optional sections:

```text
NATIVE_LINK_TABLE
RESOURCE_TABLE
DEBUG_INFO
SOURCE_MAP
AUDIT_INFO
```

A package containing `LINK` declarations must include `NATIVE_LINK_TABLE`. If a package contains resource types, including native resources, it must include `RESOURCE_TABLE`.

---

# Metadata Encoding

Metadata sections use table formats with integer indexes. Strings are stored once in `STRING_POOL` and referenced by `stringId`.

Indexes are zero-based. Invalid indexes reject the package.

## `STRING_POOL`

```text
stringCount    u32

repeated stringCount times:
  byteLength   u32
  bytes        byte[byteLength]
```

Strings are UTF-8. Invalid UTF-8 rejects the package.

The empty string is allowed.

## `MANIFEST`

```text
packageName        stringId
packageIdent       stringId
packageVersion     stringId
identKey           stringId
identFingerprint   stringId
signingFingerprint stringId
author             stringId
url                stringId

bytecodeMajor     u16
bytecodeMinor     u16
languageMajor     u16
languageMinor     u16

minimumRuntimeMajor  u16
minimumRuntimeMinor  u16

dependencyCount   u32
nativeLinkCount   u32
exportCount       u32
entryFunction     functionId or 0xFFFFFFFF
entryFlags        u32
```

The manifest identity, `identKey`, `identFingerprint`, and `signingFingerprint` must match the `.mfp` header identity, `identKey`, `identFingerprint`, and `signingFingerprint`.

`entryFunction` identifies the executable entry point when the bytecode payload is the root executable payload or has been produced by merging package bytecode into the root project bytecode. Reusable packages set it to `0xFFFFFFFF`. Entry flags:

```text
bit 0 = package has executable entry
bit 1 = entry accepts command-line args as List OF String
bit 2 = entry is FUNC returning Integer
```

The executable runtime maps `SUB` entry success to process exit code `0`, `FUNC ... AS Integer` entry success to the returned integer value, and an uncaught entry error result carrying `error` to stderr output of `error.message` plus process exit code `error.code`. When args are accepted, argument element zero is the program name as invoked by the host.

The manifest is the signed source of truth. The container header duplicates identity fields only so package managers can scan files without parsing every table.

## `IMPORT_TABLE`

Each imported package entry:

```text
importCount      u32

repeated importCount times:
  packageName    stringId
  packageIdent   stringId
  version        stringId
  pin            u8
  flags          u32
  usedSymbolCount u32

  repeated usedSymbolCount times:
    symbolName   stringId
    abiHash      byte[32]
```

Import flags:

```text
bit 0 = import contains native dependencies
```

`packageName` is the source import name used by bytecode and package-qualified names. `packageIdent` is the resolver identity `<owner>#<package>`. `version` is the requested concrete semantic version. `pin = 0` means the resolver may choose the highest ABI-compatible version anchored at `version`; `pin = 1` means the resolver must choose exactly `version`.

`usedSymbolCount` records the imported public ABI surface this package was compiled against. Each `abiHash` is the 32-byte ABI hash from the imported package's `ABI_INDEX` for `symbolName`. The resolver and bytecode merger use these hashes to prove that a selected package version still provides the imported symbols with compatible signatures.

Import graph cycles remain compile-time or bytecode merge-time errors.

## `EXPORT_TABLE`

Each exported symbol entry:

```text
exportCount       u32

repeated exportCount times:
  name            stringId
  kind            u16
  flags           u16
  targetId        u32
```

Export kinds:

```text
1 = function
2 = sub
3 = top-level LET
4 = top-level MUT
5 = type
6 = union
7 = enum
8 = union member constructor
9 = record constructor
10 = native wrapper function
```

This preserves the source-level rule that importers see package-qualified names.

## `ABI_INDEX`

The `ABI_INDEX` section records the public ABI hashes exported by this package and the imported ABI hashes this package was compiled against. It is required for every package.

```text
abiFormatVersion  u16
reserved          u16

exportAbiCount    u32

repeated exportAbiCount times:
  name            stringId
  kind            u16
  abiHash         byte[32]

dependencyAbiCount u32

repeated dependencyAbiCount times:
  packageName     stringId
  packageIdent    stringId
  version         stringId
  pin             u8
  usedSymbolCount u32

  repeated usedSymbolCount times:
    symbolName    stringId
    abiHash       byte[32]
```

`abiFormatVersion = 1` uses SHA-256 hashes. The hash input for every exported ABI item begins with `MFBABI\0`, the ABI format version, the declaration kind, the fully qualified exported name, and the declaration-specific public shape described below. The hash input must use canonical type names/ids and canonical constant encodings so two compilers produce the same ABI hash for the same public surface.

ABI v1 covers all caller-visible exported surface. It is not function-only. The required v1 declaration kinds are:

```text
1 = exported FUNC
2 = exported SUB
3 = exported record type
4 = exported union type
5 = exported enum type
6 = exported constant
7 = exported global LET
8 = exported global MUT
9 = exported native wrapper function
10 = exported resource type
```

For exported `FUNC`, `SUB`, and native wrapper functions, the hash input includes exported effect flags visible to callers, resource ownership annotations on parameters and returns, parameter count, parameter names when they are part of call syntax, parameter types, default argument presence and default constant values, return type for functions, and error/result behavior visible to callers.

For exported record types, the hash input includes the record name, type parameters if any, field count, field order, field names, field types, visibility/export flags, mutability, default presence, and default constant values. Reordering exported fields is ABI-significant.

For exported union types, the hash input includes the union name, type parameters if any, member count, member order or explicit tags, and each member type's exported identity.

For exported enum types, the hash input includes the enum name, member count, member order, member names, and explicit ordinals/discriminants. Changing an exported ordinal is ABI-significant.

For exported constants, the hash input includes the constant name, type, and canonical value when the value is visible to consumers at compile time.

For exported global `LET` and `MUT`, the hash input includes the global name, declared type, mutability, initialization visibility, and caller-visible access shape. Changing `LET` to `MUT` or `MUT` to `LET` is ABI-significant.

For exported resource types, the hash input includes the resource type name, close function signature, ownership/borrow/consume behavior, sendability flags, native/standard resource flag, and whether close may fail. These fields must agree with `RESOURCE_TABLE` and function metadata.

Future ABI index versions may add more declaration kinds or more detailed hashes, but v1 must include every exported declaration kind listed above so the resolver cannot report ABI compatibility while exported types, constants, globals, native wrappers, resource behavior, or caller-visible effects have changed.

`exportAbiCount` must match the ABI-relevant entries in `EXPORT_TABLE` and must appear in the same order. The verifier must reject an `ABI_INDEX` whose export names, kinds, order, or hashes disagree with the bytecode metadata.

`dependencyAbiCount` must match `IMPORT_TABLE` by package import name and package ident. Each dependency ABI entry repeats the requested `version` and `pin` state and records every imported symbol whose ABI shape was used while compiling this package, including imported functions/subs, exported types, constants, globals, native wrappers, resource behavior, and caller-visible effects. These hashes are also present in `IMPORT_TABLE` so tools that only need dependency requirements can read one section; `ABI_INDEX` is the canonical ABI compatibility section when the two disagree.

---

# Type Table

The `TYPE_TABLE` defines all types referenced by the package bytecode.

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
10 = standard resource
```

There are no open template declarations in package bytecode. `List`, `Map`, `Result`, and `Thread` are compiler-owned templates, user templates are expanded by the source compiler, and the type table stores only concrete instantiations such as `List OF Integer`, `Result OF Vec3`, or a user-defined `Stack OF String`.

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

---

# Constant Pool

The `CONST_POOL` stores immutable literal values.

```text
constCount      u32
ConstEntry[constCount]
```

## Constant entry

```text
kind            u16
flags           u16
payloadLength   u32
payload         byte[payloadLength]
```

Constant kinds:

```text
1 = Nothing
2 = Boolean
3 = Integer
4 = Float
5 = Fixed
6 = String
7 = Byte
8 = Error
```

Encoding:

```text
Nothing  payloadLength = 0
Boolean  u8, 0 = FALSE, 1 = TRUE
Integer  i64
Float    u64 IEEE-754 binary64 bit pattern
Fixed    i64 raw signed 32/32 fixed-point value
String   stringId as u32
Byte     u8
Error    code i64, message stringId
```

Float constants must use canonical quiet NaN representation if NaN constants are ever allowed. Implementations may reject NaN constants in source if deterministic behavior is not yet specified.

---

# Globals

The `GLOBAL_TABLE` stores top-level `LET` and `MUT` bindings.

```text
globalCount     u32

repeated globalCount times:
  name          stringId
  type          typeId
  flags         u32
  initFunction  functionId or 0xFFFFFFFF
```

Global flags:

```text
bit 0 = exported
bit 1 = mutable
bit 2 = initialized by constant
bit 3 = initialized by function
```

A package may have a package initializer function. The bytecode merger records package initializers in dependency order so the executable runtime can run them before `main`. Isolated thread package instances run their own package initializers when the thread package instance starts.

---

# Functions

The `FUNCTION_TABLE` stores all bytecode functions, native wrapper functions, imported function references, and package initializer functions.

```text
functionCount   u32
FunctionEntry[functionCount]
```

## Function entry

```text
name            stringId
kind            u16
flags           u16

paramCount      u32
returnType      typeId

registerCount   u32
codeOffset      u64
codeLength      u64

trapPc          u32
cleanupCount    u32
cleanupOffset   u64
```

Function kinds:

```text
1 = bytecode function
2 = imported function
3 = native wrapper function
4 = built-in function reference
5 = package initializer
```

Function flags:

```text
bit 0 = exported
bit 1 = private
bit 2 = isolated
bit 3 = sub
bit 4 = hasTrap
bit 5 = returnsNothingOnSuccess
```

The `returnType` is the declared success type. The effective runtime result is always `Result OF returnType`, consistent with the language rule that every function returns `Result` and call sites auto-unwrap or auto-propagate unless directly matched. 

If `hasTrap` is false, `trapPc` must be `0xFFFFFFFF`.

## Parameters

Immediately following each `FunctionEntry` or in an associated payload table:

```text
paramName       stringId
paramType       typeId
paramFlags      u32
defaultConst    constId or 0xFFFFFFFF
```

Parameter flags:

```text
bit 0 = has default
bit 1 = resource borrow
bit 2 = resource consume
```

No `BORROW` or `MOVE` source syntax is required. These are compiler/runtime metadata rules.

---

# Code Section

The `CODE` section contains instruction streams for bytecode functions.

A function’s `codeOffset` and `codeLength` point into the `CODE` section.

## Function code layout

```text
instructionCount     u32
Instruction[instructionCount]
```

Program counters are instruction indexes, not byte offsets. Branch targets refer to instruction indexes within the same function.

## Instruction encoding

```text
opcode          u16
flags           u16
operandCount    u16
reserved        u16
operands        u32[operandCount]
```

All large values are loaded from the constant pool. Operands are indexes into registers, constants, types, fields, union members, functions, globals, or instruction targets depending on the opcode.

This format is intentionally simple and verifier-friendly. A future compact encoding may be added under a new `bytecodeMajor` or `bytecodeMinor`.

---

# Registers

Each function has a fixed number of virtual registers.

Registers are typed. Register types are stored in function metadata:

```text
registerCount     u32

repeated registerCount times:
  registerType    typeId
  registerFlags   u32
```

Register flags:

```text
bit 0 = parameter register
bit 1 = mutable local cell
bit 2 = resource register
bit 3 = initialized at entry
```

The verifier tracks whether each register is initialized at every program point.

The verifier also tracks resource ownership. Resource registers cannot be copied. A resource register is either:

```text
uninitialized
owned
borrowed
moved
closed
```

---

# Core Instructions

## No-op

```text
NOP
```

## Constants and defaults

```text
LOAD_CONST      dst, constId
LOAD_DEFAULT    dst, typeId
```

## Movement and ownership

```text
MOVE            dst, src
COPY            dst, src
DROP            src
FREEZE          dst, src
```

Rules:

* `COPY` is valid only for copyable values.
* `MOVE` transfers ownership and marks `src` moved.
* Reading a moved register is a verifier error.
* `FREEZE` converts a locally mutable collection buffer into an immutable owned value.
* `DROP` releases a non-resource value or marks a register dead.
* Resource values are not dropped silently; they must be closed, moved, returned, or owned by an active `USING`.

## Global access

```text
LOAD_GLOBAL     dst, globalId
STORE_GLOBAL    globalId, src
```

`STORE_GLOBAL` is valid only for top-level `MUT` globals.

## Built-in IO and filesystem

```text
IO_WRITE          dst, stringReg, fdConst, appendNewlineConst
IO_FLUSH          dst, fdConst
IO_READ_LINE      dst, promptRegOrU32Max
IO_READ_CHAR      dst
IO_READ_BYTE      dst
IO_IS_TERMINAL    dst, fdConst
IO_TERMINAL_SIZE  dst
IO_OPEN           dst, pathReg, modeReg
IO_CLOSE          dst, fileHandleReg
```

`IO_OPEN` and `IO_CLOSE` are portable bytecode operations. `pathReg` and `modeReg` are `String` registers, `modeReg` contains a source-level portable mode string, and `dst` for `IO_OPEN` has built-in type `File`. Bytecode must not encode host constants such as POSIX `O_RDONLY`, Darwin syscall numbers, Windows access masks, or libc symbol names. Native backends lower these operations to the target runtime helper contract below.

## Records

```text
MAKE_RECORD     dst, typeId, fieldReg...
GET_FIELD       dst, src, fieldIndex
WITH_FIELD      dst, src, fieldIndex, value
```

## Unions

```text
MAKE_MEMBER         dst, unionTypeId, memberIndex, valueReg
MEMBER_TAG          dst, src
GET_MEMBER_VALUE    dst, src, memberIndex
```

`GET_MEMBER_VALUE` is valid only on control-flow paths where the verifier knows the active union member matches, or immediately after a checked branch.

## Enums

```text
LOAD_ENUM       dst, enumTypeId, ordinal
```

## Lists and maps

These are used primarily for literals and compiler-generated construction. Most collection operations remain normal calls to built-in functions.

```text
LIST_NEW        dst, listTypeId, capacityConst
LIST_PUSH       listReg, itemReg

MAP_NEW         dst, mapTypeId, capacityConst
MAP_PUT         mapReg, keyReg, valueReg
```

The verifier enforces element/key/value types.

## Arithmetic and comparison

```text
NEG             dst, a
ADD             dst, a, b
SUB             dst, a, b
MUL             dst, a, b
DIV             dst, a, b
MOD             dst, a, b
POW             dst, a, b

EQ              dst, a, b
NE              dst, a, b
LT              dst, a, b
LE              dst, a, b
GT              dst, a, b
GE              dst, a, b

AND             dst, a, b
OR              dst, a, b
XOR             dst, a, b
NOT             dst, a

CONCAT          dst, a, b
```

Arithmetic instructions use MFBASIC checked semantics. If an operation fails, for example due to overflow or divide-by-zero, it creates an `Error` and routes to the active trap or returns `Err`. In the current compiler/runtime source, numeric runtime errors use the same runtime codes documented in `specifications/error_codes.md`, for example `ErrOverflow = 77050010`.

Short-circuiting `AND` and `OR` are normally compiled with branches rather than relying on the `AND`/`OR` opcodes.

## Control flow

```text
JMP             targetPc
JMP_TRUE        condReg, targetPc
JMP_FALSE       condReg, targetPc
```

Branch targets must be valid instruction indexes.

The verifier must reject jumps into trap blocks, cleanup regions, or the middle of compiler-generated structured regions.

## Function calls and `Result`

Every call produces a raw `Result` value in bytecode. Source-level auto-unwrapping is compiled as a call followed by `UNWRAP_RESULT`.

```text
CALL_RESULT     dstResult, functionId, argReg...
UNWRAP_RESULT   dstValue, resultReg

MAKE_OK         dstResult, valueReg
MAKE_ERR        dstResult, errorReg

RESULT_IS_OK    dstBool, resultReg
RESULT_VALUE    dstValue, resultReg
RESULT_ERROR    dstError, resultReg
```

Source:

```basic
LET x = toInt(s)
```

Bytecode pattern:

```text
CALL_RESULT     r1, toInt, r0
UNWRAP_RESULT   r2, r1
```

Source direct `MATCH`:

```basic
MATCH toInt(s)
  CASE Ok(n)  : ...
  CASE Error(e) : ...
END MATCH
```

Bytecode pattern:

```text
CALL_RESULT     r1, toInt, r0
RESULT_IS_OK    r2, r1
JMP_FALSE       r2, errCase
RESULT_VALUE    r3, r1
...
JMP             endMatch
errCase:
RESULT_ERROR    r4, r1
...
endMatch:
```

This keeps the language clean while making the bytecode explicit and auditable.

## Errors and traps

```text
MAKE_ERROR      dstError, codeReg, messageReg
FAIL            errorReg
PROPAGATE
RETURN_OK       valueReg
RETURN_ERR      errorReg
```

Rules:

* `RETURN_OK` returns the success member carrying `value`.
* `RETURN_ERR` returns the error member carrying `error`.
* `FAIL` transfers to the function trap if one exists; otherwise it returns the error member carrying `error`.
* `PROPAGATE` is valid only in trap code.
* `UNWRAP_RESULT` behaves like `FAIL` when the result is the error member.

The function table’s `trapPc` gives the single bottom trap entry point.

## Resources and `USING`

```text
USING_ENTER     resourceReg, closeFunctionId, cleanupId
USING_LEAVE     cleanupId
CLOSE_RESOURCE  resourceReg, closeFunctionId
```

Rules:

* `USING_ENTER` registers a resource as owned by the current lexical `USING` region.
* `USING_LEAVE` closes the resource exactly once.
* `CLOSE_RESOURCE` is compiler-generated for explicit close operations or `USING` lowering.
* If control exits a `USING` region through `FAIL`, `UNWRAP_RESULT`, `RETURN_OK`, `RETURN_ERR`, or branch, the bytecode must either close the resource explicitly or have cleanup metadata that closes it.
* The verifier rejects paths where an owned resource can be lost, copied, double-closed, used after close, or read after move.

The existing resource model already says files, sockets, and similar handles are scoped with `USING` and closed deterministically, including on error exits.  The bytecode makes that rule verifiable.

---

# Cleanup Table

Each bytecode function may contain cleanup metadata.

```text
cleanupCount      u32

repeated cleanupCount times:
  cleanupId       u32
  startPc         u32
  endPc           u32
  resourceReg     u32
  closeFunctionId u32
```

A cleanup region is active for instruction indexes:

```text
startPc <= pc < endPc
```

Verifier rules:

* A cleanup region must begin at or after `USING_ENTER`.
* A cleanup region must end at or before `USING_LEAVE`.
* Control may not jump into a cleanup region from outside.
* Control may leave a cleanup region only through paths that close the resource or through runtime cleanup transfer.
* `closeFunctionId` must accept the exact resource type.

---

# Native Binding Metadata

A package containing `LINK` declarations is still a normal `.mfp` package. The application imports it normally. The binding metadata lives inside the signed bytecode payload.

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
* Native symbols are whitelisted by this table. MFBASIC bytecode cannot perform dynamic native symbol lookup.

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

---

# Verifier Rules

The `.mfp` verifier runs before a package can be imported or merged.

The verifier must reject malformed, unsafe, or incompatible packages before any package code runs.

Current compiler source of truth:

- Verification and package-read failures are currently surfaced as detailed package/container validation messages from the package reader and verifier implementation, not as a single emitted `rules.rs` diagnostic family.
- The spec should therefore treat the concrete rejection conditions below as normative for current behavior, with message text such as invalid magic, unsupported version, invalid signature header, truncated section table, missing section, identity mismatch, or other malformed-container diagnostics.

## Container verifier

The container verifier checks:

* Magic bytes.
* Container version.
* Bytecode version.
* Signature type and signature length.
* Signature validity.
* Header string validity.
* Exact `bytecodeLength`.
* No trailing bytes.
* Header identity, identKey, identFingerprint, and signingFingerprint match manifest identity, identKey, identFingerprint, and signingFingerprint.

## Section verifier

The section verifier checks:

* Required sections exist.
* No duplicate required sections.
* Section offsets are in range.
* Section ranges do not overlap.
* Section payloads parse exactly.
* Unknown required sections reject the package.
* Optional unknown sections may be ignored only if their flags permit ignoring.

## Type verifier

The type verifier checks:

* All `typeId` references are valid.
* No open template declarations exist.
* Template instantiations are concrete.
* `Map` keys are comparable.
* Union member indexes are valid.
* Record field indexes are valid.
* Function types have valid parameter and return types.
* `CPtr` does not appear in ordinary MFBASIC type signatures.

## Function verifier

The function verifier checks:

* All registers have valid types.
* All registers are initialized before read.
* All instructions have valid operands.
* Branch targets are valid instruction indexes.
* No jump enters a trap block from outside except through error routing.
* No jump enters a cleanup region from outside.
* All function paths return `Ok` or `Err`.
* `PROPAGATE` appears only in trap code.
* `UNWRAP_RESULT` operates only on `Result` registers.
* `RESULT_VALUE` is used only on proven-`Ok` paths.
* `RESULT_ERROR` is used only on proven-`Err` paths.
* Calls pass the correct number and type of arguments.
* Isolated function restrictions are preserved.

## Resource verifier

The resource verifier checks:

* Resource values are never copied.
* Resource values are not compared, printed, serialized, or stored in ordinary collections.
* Resource values are not captured by lambdas.
* Resource values are not sent to threads unless explicitly marked sendable.
* A resource is not used after close.
* A resource is not used after move.
* A resource is closed exactly once when owned by `USING`.
* A resource returned from a function transfers ownership to the caller.
* A resource passed to a consuming close function is marked closed afterward.
* A borrowed resource cannot outlive the call that borrowed it.

## Native verifier

The native verifier checks:

* All native libraries referenced by metadata are declared.
* All native symbols are declared in `NATIVE_LINK_TABLE`.
* Native wrapper function signatures match their ABI entries.
* `CString` use is explicit.
* `OUT` and `REF` lifetimes do not escape.
* `CPtr` does not escape the native boundary.
* Resource ownership is declared through `RESOURCE_TABLE`.
* A package containing native metadata sets the container native flag.

This directly addresses the `.mfp` verifier gap identified in the review: type-checked bytecode, initialized register use, resource ownership, valid control flow, package signature validation, and native-link manifest validation. 

---

# Minimal Example Layout

A small package:

```basic
EXPORT FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC
```

Produces conceptually:

```text
MFPHeader
  magic
  versions
  signatureType
  signatureLength
  signature
  name = "mathstuff"
  ident = "ada#mathstuff"
  version = "1.0.0"
  identKey = "ed25519-public:..."
  identFingerprint = "sha256:..."
  signingFingerprint = "sha256:..."
  author = "..."
  url = "..."
  bytecodeLength = N

packageBytecode
  BytecodeHeader
  MANIFEST
  STRING_POOL
    "mathstuff"
    "ada#mathstuff"
    "1.0.0"
    "ed25519-public:..."
    "sha256:..."
    "sha256:..."
    "add"
    "a"
    "b"
  TYPE_TABLE
    Integer references built-in type id 3
  CONST_POOL
    empty
  IMPORT_TABLE
    empty
  EXPORT_TABLE
    add -> function 0
  ABI_INDEX
    add -> SHA-256 ABI hash
  GLOBAL_TABLE
    empty
  FUNCTION_TABLE
    function 0: add(Integer, Integer) AS Integer
  CODE
    LOAD / ADD / RETURN_OK instruction stream
```

The function body could lower to:

```text
ADD        r2, r0, r1
RETURN_OK  r2
```

If `ADD` overflows, it creates `ErrOverflow` (`77050010`) and routes to the trap or returns `Err`, depending on the function metadata.

---

# Pasteable short spec addition

This is the compact version I would add to your current `Build Artifacts` section:

````markdown
### `.mfp` Container Format

A `.mfp` package is a signed binary container followed by architecture-independent MFB bytecode.

All integers are little-endian. All strings are UTF-8 byte strings with a `u32` byte length. No strings are NUL-terminated.

The container header is:

```text
magic              8 bytes
containerMajor     u16
containerMinor     u16
bytecodeMajor      u16
bytecodeMinor      u16
flags              u32

signatureType      u16
signatureLength    u32
signature          byte[signatureLength]

nameLength         u32
name               byte[nameLength]

identLength        u32
ident              byte[identLength]

versionLength      u32
version            byte[versionLength]

identKeyLength     u32
identKey           byte[identKeyLength]

identFingerprintLength u32
identFingerprint       byte[identFingerprintLength]

signingFingerprintLength u32
signingFingerprint       byte[signingFingerprintLength]

authorLength       u32
author             byte[authorLength]

urlLength          u32
url                byte[urlLength]

bytecodeLength     u64

packageBytecode    byte[bytecodeLength]
````

The package content hash and package signature use the entire `.mfp` file with only the `signature` byte range replaced by zero bytes of the same length:

```text
signatureStart = 26
signatureEnd   = signatureStart + signatureLength
coveredBytes   = file[0 : signatureStart] || zero[signatureLength] || file[signatureEnd : end]
contentHash    = SHA-256(coveredBytes)
signatureInput = "MFP-PACKAGE-v1" || contentHash || ident || version
```

This covers the magic, container version, bytecode version, flags, signature type, signature length, header metadata, bytecode length, and bytecode. It excludes only the actual signature bytes.

`signatureType = 0` means unsigned and requires `signatureLength = 0`. `signatureType = 1` means Ed25519 and requires `signatureLength = 64`. Unknown signature types reject the package. Public registry packages must use `signatureType = 1`; installs reject unsigned packages except for explicit `allowUnsignedLocal` exceptions on `path:` or `file:` sources.

The bytecode payload must contain a signed package manifest. The manifest package name, ident, version, identKey, identFingerprint, and signingFingerprint must match the header package name, ident, version, identKey, identFingerprint, and signingFingerprint.

### Package Bytecode

The package bytecode begins with:

```text
bcMagic        4 bytes = "MFBC"
bcMajor        u16
bcMinor        u16
bcFlags        u32
sectionCount   u32
sectionTable   SectionHeader[sectionCount]
sectionData    byte[]
```

Each section header is:

```text
sectionId      u16
sectionFlags   u16
reserved       u32
offset         u64
length         u64
```

Required sections are:

```text
MANIFEST
STRING_POOL
TYPE_TABLE
CONST_POOL
IMPORT_TABLE
EXPORT_TABLE
GLOBAL_TABLE
FUNCTION_TABLE
CODE
ABI_INDEX
```

Optional sections are:

```text
NATIVE_LINK_TABLE
RESOURCE_TABLE
DEBUG_INFO
SOURCE_MAP
AUDIT_INFO
```

The bytecode is a typed register bytecode. It contains no machine code, native addresses, host pointers, or platform-specific object layouts. Branch targets are bytecode instruction indexes. Constants, strings, types, imports, exports, globals, functions, native bindings, and resources are referenced by table indexes.

Every function returns `Result` at the bytecode level. Source-level auto-unwrapping is compiled as `CALL_RESULT` followed by `UNWRAP_RESULT`. A direct `MATCH` on a call compiles as `CALL_RESULT` followed by explicit `Result` inspection.

The verifier must check section bounds, type references, initialized register use, valid branch targets, valid trap control flow, resource ownership, native binding metadata, and package signature validity before the package may be imported or merged.

```

That gives you a concrete `.mfp` container and a sane bytecode foundation without turning MFBASIC into a giant VM spec too early.
```

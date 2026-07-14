# `.mfp` Package Format

A `.mfp` file is a signed MFBASIC package. It contains:

```text
MFP container header
MFB architecture-independent Binary Representation
```

The container header provides quick package identity and signature information. The **Binary Representation** payload contains the package manifest, dependency metadata, public API metadata, type tables, constants, functions, native binding declarations, and the structured encoding that carries every function body.

The **Binary Representation** is a compact, **versioned, structured** binary encoding of a compiled program. It is *not* a flat register/stack machine: there is no opcode ISA, no `JMP`/`JMP_FALSE`, and no program counter. Control flow stays nested (regions with explicit ends) and expressions stay as trees. A consumer **decodes** the Binary Representation and lowers it through the single codegen path used for the executable's own code, so package functions get every language feature for free. (The Binary Representation is a versioned external serialization of the compiler's internal program model; see `architecture.md` for how the two relate.)

All integers in `.mfp` files are little-endian. All strings are UTF-8 byte strings and are length-prefixed. No field is NUL-terminated.

## Container layout

```text
.mfp file
  MFPHeader
  packageBinaryRepr
```

## `MFPHeader`

```text
magic              8 bytes
containerMajor     u16
containerMinor     u16
binaryReprMajor      u16
binaryReprMinor      u16
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

binaryReprLength     u64

packageBinaryRepr    byte[binaryReprLength]
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
| `binaryReprMajor`   | Required major version of the package Binary Representation format. Currently `2`. |
| `binaryReprMinor`   | Required minor version of the package Binary Representation format.            |
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
| `binaryReprLength`  | Exact byte length of `packageBinaryRepr`.                           |
| `packageBinaryRepr` | Architecture-independent MFB Binary Representation image. |

The header `name`, `ident`, `version`, `identKey`, `identFingerprint`, `signingFingerprint`, `author`, and `url` are for fast package scanning. The binary representation payload must also contain a signed manifest with the same package identity, owner ident key, owner ident fingerprint, and signing fingerprint. A verifier must reject the package if the header identity and binary representation manifest identity do not match.

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
binaryReprMajor
binaryReprMinor
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
binaryReprLength
packageBinaryRepr
```

The covered bytes exclude only the actual signature bytes:

```text
signature
```

This signs the package import name, registry ident, owner ident key, owner ident
fingerprint, signing fingerprint, version, container format versions, binary representation
format versions, flags, metadata, and binary representation. `binaryReprLength` is covered, so
truncation, extension, or binary representation replacement invalidates the signature.

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

- Package/container rejection currently comes from detailed package-reader diagnostics in `src/binary_repr.rs`, `src/target/package_mfp/mod.rs`, and `src/main.rs`.
- These failures are currently surfaced as descriptive `error: ...` strings such as invalid magic, invalid signature header, truncated signature, or unsupported binary representation/container version rather than through a single package rule code path.

## Container validation

A reader must reject an `.mfp` package when:

* `magic` does not match. The current compiler reports this as `package does not have the MFP package magic`.
* `containerMajor` is unsupported. The current compiler reports this as `unsupported MFP container major version <n>`.
* `binaryReprMajor` is unsupported. The package Binary Representation format is now at major version `2`; this is a **clean break** from the old flat opcode payload (major `1`). A reader rejects any package that predates the structured Binary Representation format. The current compiler reports this as `unsupported MFPC major version <n> (expected 2); this package predates the structured Binary Representation format and must be rebuilt`.
* `signatureType` is unknown. The current compiler reports this as `unsupported .mfp signature type <n>`.
* `signatureLength` is invalid for the signature type. The current compiler reports either `unsigned .mfp package must have zero signature length` or `Ed25519 .mfp package must have a 64 byte signature`.
* The signature fails verification under the selected trust policy.
* Any string length exceeds the implementation limit.
* `binaryReprLength` does not exactly match the remaining byte count. The current compiler reports this as `invalid .mfp binary representation length`.
* There are trailing bytes after `packageBinaryRepr`.
* The container header identity does not match the embedded binary representation manifest identity. The current compiler reports this as `MFP header identity does not match binary representation manifest identity`.
* The binary representation manifest package name, ident, version, identKey, identFingerprint, or signingFingerprint do not match the header name, ident, version, identKey, identFingerprint, or signingFingerprint.

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
binaryReprLength            <= implementation-defined maximum
```

Package names should use the same identifier restrictions as source package names unless the package manager later defines a wider registry naming scheme.

---

# MFB Package Binary Representation

The package binary representation is the architecture-independent payload stored after the `.mfp` header.

The binary representation is not machine code. It contains no native addresses, host pointers, host object layouts, CPU instructions, or platform-specific calling conventions. It is the **structured Binary Representation** — a faithful, versioned serialization of the compiled program — plus the metadata tables that describe the package.

The package container format is called **MFPC**. Its container major version is **2** (the clean break to the structured Binary Representation; the old flat opcode payload was major `1` and is rejected outright).

The Binary Representation is *not* a flat opcode stream: control flow is encoded as nested regions with explicit ends (`IF/THEN/ELSE/END`, `WHILE/DO/END`, `FOREACH/IN/DO/END`, `MATCH/CASE/.../END`, `TRAP/.../END`) and expressions stay as trees (`Binary`, `Call`, `CallResult`, `ResultIsOk/Value/Error`, `Constructor`, `MemberAccess`, literals, identifiers, …). A reader walks the tree; structure is read, never reconstructed from jumps. This is the same principle WebAssembly uses (structured control flow, no arbitrary jumps), kept at MFBASIC's own semantic level so the encoding still knows `List`, `Map`, `Result`, owned `File`, and threads.

```text
packageBinaryRepr
  BinaryReprHeader
  SectionTable
  SectionData...
```

## Binary Representation header

```text
bcMagic        4 bytes
bcMajor        u16   = 2 (structured Binary Representation; major 1 was the old flat payload and is rejected)
bcMinor        u16
bcFlags        u32
sectionCount   u32
sectionTable   SectionHeader[sectionCount]
sectionData    byte[]
```

Recommended `bcMagic`:

```text
4D 46 50 43
M  F  P  C
```

## Section header

```text
sectionId      u16
sectionFlags   u16
reserved       u32
offset         u64
length         u64
```

`offset` is relative to the start of `packageBinaryRepr`, not the start of the file.

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
10 = NATIVE_LINK_TABLE
11 = RESOURCE_TABLE
12 = DEBUG_INFO
13 = SOURCE_MAP
14 = AUDIT_INFO
15 = ABI_INDEX
16 = IR
17 = DOC
```

Section id `9` (the old flat `CODE` stream) is **retired**. Function bodies are now carried by the `IR` section (id `16`) as structured Binary Representation; the function table records zero-length code regions.

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
IR
ABI_INDEX
```

Optional sections:

```text
NATIVE_LINK_TABLE
RESOURCE_TABLE
DEBUG_INFO
SOURCE_MAP
AUDIT_INFO
DOC
```

A package containing `LINK` declarations must include `NATIVE_LINK_TABLE`. If a package contains resource types, including native resources, it must include `RESOURCE_TABLE`. A package with at least one exported `DOC` block (or a `PACKAGE` doc block) includes the optional `DOC` section.

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

binaryReprMajor     u16
binaryReprMinor     u16
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

`entryFunction` identifies the executable entry point when the binary representation payload is the root executable payload or has been produced by merging package binary representation into the root project binary representation. Reusable packages set it to `0xFFFFFFFF`. Entry flags:

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

`packageName` is the source import name used by binary representation and package-qualified names. `packageIdent` is the resolver identity `<owner>#<package>`. `version` is the requested concrete semantic version. `pin = 0` means the resolver may choose the highest ABI-compatible version anchored at `version`; `pin = 1` means the resolver must choose exactly `version`.

`usedSymbolCount` records the imported public ABI surface this package was compiled against. Each `abiHash` is the 32-byte ABI hash from the imported package's `ABI_INDEX` for `symbolName`. The resolver and binary representation merger use these hashes to prove that a selected package version still provides the imported symbols with compatible signatures.

Import graph cycles remain compile-time or binary representation merge-time errors.

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

`exportAbiCount` must match the ABI-relevant entries in `EXPORT_TABLE` and must appear in the same order. The verifier must reject an `ABI_INDEX` whose export names, kinds, order, or hashes disagree with the binary representation metadata.

`dependencyAbiCount` must match `IMPORT_TABLE` by package import name and package ident. Each dependency ABI entry repeats the requested `version` and `pin` state and records every imported symbol whose ABI shape was used while compiling this package, including imported functions/subs, exported types, constants, globals, native wrappers, resource behavior, and caller-visible effects. These hashes are also present in `IMPORT_TABLE` so tools that only need dependency requirements can read one section; `ABI_INDEX` is the canonical ABI compatibility section when the two disagree.

---

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

A package may have a package initializer function. The binary representation merger records package initializers in dependency order so the executable runtime can run them before `main`. Isolated thread package instances run their own package initializers when the thread package instance starts.

---

# Functions

The `FUNCTION_TABLE` stores all functions, native wrapper functions, imported function references, and package initializer functions. The table *describes* each function (name, signature, kind, flags, parameters, declared return/effect); the function *body* is the structured Binary Representation carried in the `IR` section.

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

codeOffset      u64
codeLength      u64
```

Because function bodies are carried by the `IR` section as structured Binary Representation, the function table records **zero-length** code regions (`codeOffset`/`codeLength` are retained for layout compatibility and are zero). There are no register tables, program counters, `trapPc`, or cleanup tables in the function entry: those flat-machine concepts do not exist in the structured form. A function's `IF`/`WHILE`/`FOREACH`/`MATCH`/`TRAP` structure, its resource regions, and its single bottom trap are all represented directly as nested Binary Representation nodes.

Function kinds:

```text
1 = binary representation function (structured Binary Representation body)
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
bit 5 = returnsNothingOnSuccess
```

The `returnType` is the declared success type. The effective runtime result is always `Result OF returnType`, consistent with the language rule that every function returns `Result` and call sites auto-unwrap or auto-propagate unless directly matched. Whether a function contains a trap is read directly from its Binary Representation body (a `Trap` region), not from a flag/PC pair.

## Parameters

Each function records its parameters (name, type, ownership annotations, default presence and default constant). Parameters with defaults carry the default value; ownership annotations record borrow/consume behavior.

Parameter flags:

```text
bit 0 = has default
bit 1 = resource borrow
bit 2 = resource consume
```

No `BORROW` or `MOVE` source syntax is required. These are compiler/runtime metadata rules, and they round-trip through the Binary Representation.

---

# Binary Representation Section

The `IR` section (id `16`) carries the structured Binary Representation payload — the faithful, versioned serialization of the project's IR functions. It replaces the retired flat `CODE` stream as the carrier of every function body.

## Payload header

```text
magic        4 bytes = "MFBR"
version      u16
IrProject    ...
```

Recommended `magic`:

```text
4D 46 42 52
M  F  B  R
```

The Binary Representation `version` is currently `1`. A reader rejects any payload whose magic is not `MFBR` or whose version it does not support (the package binary representation container is separately versioned at MFPC major `2`).

The payload is self-contained: integers are little-endian, strings are inline length-prefixed (`u32` byte length followed by UTF-8 bytes). The in-memory IR is free to change behind this format; the encoding is the stable contract, and `IR → Binary Representation → IR` is an identity round-trip across every node kind.

## Structured control flow (no jumps)

Control flow is encoded as nested regions with explicit ends, matching IR exactly:

```text
IF      <cond-expr> THEN <ops...> ELSE <ops...> END
WHILE   <cond-expr> DO <ops...> END
DO      <ops...> UNTIL <cond-expr>
FOR     <name> = <start> TO <end> STEP <step> DO <ops...> END
FOREACH <name> IN <iterable-expr> DO <ops...> END
MATCH   <scrutinee-expr> CASE <pattern> [<guard>] <ops...> ... [ELSE <ops...>] END
TRAP    <binding> <ops...> END
```

Structured exit out of these regions is itself encoded as leaf ops rather than jumps: `ExitLoop` (`EXIT FOR/DO/WHILE`), `ContinueLoop` (`CONTINUE FOR/DO/WHILE`), and `ExitProgram` (`EXIT PROGRAM`). There are no `JMP`, `JMP_FALSE`, label, or program-counter concepts in the format. A reader walks the tree; structure is read, never reconstructed.

## Statements / ops

`IrOp` is encoded faithfully, one tag byte per kind. The kinds are `Bind`, `Assign`, `AssignGlobal`, `Return`, `Fail`, `Eval`, the structured control-flow regions above (`If`, `Match`, `While`, `For`, `DoUntil`, `ForEach`, `Trap`), and the structured exit ops `ExitLoop`, `ContinueLoop`, and `ExitProgram`. Source-level `PROPAGATE` and `RECOVER` are lowered before serialization (`PROPAGATE` becomes `Fail`; `RECOVER` is lowered into ordinary ops), so they are not distinct Binary Representation ops. There are no resource ops: resource lifetime is implicit (see “Resource regions” below). The internal `Result`/`Ok` forms remain implementation-only — they appear in IR and therefore in Binary Representation, but are never user-visible.

## Expressions stay nested

`IrValue` is encoded as a tree, one tag byte per kind: `Binary { op, left, right }`, `Call { target, args }`, `CallResult { … }`, `ResultIsOk` / `ResultValue` / `ResultError`, `Constructor`, `MemberAccess`, `UnionWrap` / `UnionExtract`, literals, and identifiers. There is no flattening into per-register temporaries. `CallResult` of a built-in is just an `IrValue::CallResult` node — there is no flat built-in dispatch, so the old "unknown function" emitter failure cannot occur, and an inline `TRAP` over a built-in serializes like any other expression.

## Tables and references

The Binary Representation rides alongside the container's interned tables (strings, types, constants, globals, imports, exports). IR nodes that reference declarations resolve against those tables. Concrete type instantiations (such as `List OF Integer` or `Result OF Out`) appear in the `TYPE_TABLE`; the Binary Representation references them.

## Consumption

A consumer **decodes** each imported package's `IR` section back into IR functions, applies the package identity prefix (`<id>.package.symbol`) as a link-time rename of every definition and reference, merges the package's types/constants/globals into the project, and lowers **everything** through the single `IR → NIR → native` path. There is no separate package binary representation→native bridge: package functions get every language feature — control flow, function-level and inline `TRAP`, all built-ins, inline-`TRAP`-on-built-in — for free, because they ride the same codegen as the executable's own code.

`<id>` is a **deterministic content hash** of the package's identity (its header `name`, `version`, and `ident`) and its binary representation payload — never a per-build random value. Because it is content-addressed, the same package reached through two dependency paths produces the same `<id>` and de-duplicates to a single merged copy, while two distinct packages that happen to share a name receive different `<id>`s and stay separate instead of colliding. The prefix is applied by the *consumer* at merge time as a consistent rename of the package's definitions **and** of every reference to them (from the executable and from other packages).

---

# Resource regions

Resource lifetime is represented implicitly by the lexical scope of the binding that owns the resource — not by explicit resource ops and not by a side cleanup table. There is **no** `RESOURCE_ENTER`, `RESOURCE_LEAVE`, or `RESOURCE_CLOSE` op in the Binary Representation.

* A resource is owned by the `Bind` that introduces it and lives for the lexical extent of the region (function body, loop body, branch, or trap) that contains that `Bind`.
* When the owning region exits, a compiler-generated lexical drop closes the resource exactly once if it is still owned. The drop is keyed off the binding's resource type and the close function recorded in `RESOURCE_TABLE`; it is not itself encoded as an op.
* An explicit close (the resource's declared consuming close operation) marks the binding moved, so the lexical drop does not close it again.
* Because regions are nested in the IR tree, every structured exit path — fall-through, `Return`, `Fail`, `ExitLoop`, `ContinueLoop`, `ExitProgram`, and trap routing — is bounded by the enclosing region; there are no PC ranges to reconstruct and no "jump into a cleanup region" to reject.

The resource model closes files, sockets, and similar handles by lexical drop when their owning binding leaves scope. The structured Binary Representation makes that rule directly verifiable from each binding's type and scope. (User-defined source resources reuse this same implicit-drop model; see `plan-resource.md`.)

---

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

---

# DOC Section

The optional `DOC` section (id `17`) carries the package's documentation surface
(plan-09-doc.md §5). It is self-contained: all strings are stored inline as
`u32`-length-prefixed UTF-8, independent of `STRING_POOL`. It does not contribute
to the ABI hash. The compiler emits it only when the package has at least one
exported `DOC` block or a `PACKAGE` doc block.

A `Prose` list is a `u32` count followed by that many `(u8 kind, str text)`
blocks, where `kind` is `0`=paragraph, `1`=warning, `2`=info, `3`=security. The
blocks render in order, interleaving paragraphs and callouts.

```text
u8                         hasPackage (0 or 1)
if hasPackage:
  str                      packageName
  Prose                    description (paragraphs + callouts)
  u8 + (str if 1)          deprecated flag, then optional message
u32                        declCount
declCount * DocEntry

DocEntry:
  u16                      kind (0=func, 1=sub, 2=type, 3=union, 4=enum)
  str                      name
  str                      signature (rendered source-form declaration line)
  str                      group ("" if none; FUNC/SUB only)
  Prose                    description (paragraphs + callouts)
  u32 + (str,str)*         args  (name, description)
  u32 + (str,str)*         props (name, description)
  str                      return description ("" if none)
  u32 + (str,str)*         errors (code, description)
  str                      example source ("" if none)
  u8                       internal (1 = exported-but-not-public)
  u8 + (str if 1)          deprecated flag, then optional message
```

`str` is a `u32` byte length followed by that many UTF-8 bytes. A consumer that
does not recognize section id `17` skips it; doc data never affects execution.

# Verifier Rules

The `.mfp` verifier runs before a package can be imported or merged.

The verifier must reject malformed, unsafe, or incompatible packages before any package code runs.

Verification operates on **decoded IR**, not a flat opcode stream. The structured form is easier to verify — structure is explicit, so there is no CFG reconstruction and no "reject jumps into trap/cleanup regions." Most invariants reuse the compiler's existing IR-level passes (type checking, ownership/resource linearity, exhaustiveness, return/effect agreement) rather than a parallel flat-binary representation verifier.

Current compiler source of truth:

- Verification and package-read failures are currently surfaced as detailed package/container validation messages from the package reader and verifier implementation, not as a single emitted `rules.rs` diagnostic family.
- The spec should therefore treat the concrete rejection conditions below as normative for current behavior, with message text such as invalid magic, unsupported version, invalid signature header, truncated section table, missing section, identity mismatch, or other malformed-container diagnostics.

## Container verifier

The container verifier checks:

* Magic bytes.
* Container version.
* Binary Representation (Binary Representation) version — MFPC major must be `2`; the old flat payload (major `1`) is rejected.
* Signature type and signature length.
* Signature validity.
* Header string validity.
* Exact `binaryReprLength`.
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

## Function verifier (IR-level)

The function verifier checks the decoded Binary Representation of each function:

* Every IR node is type-correct — operands, calls, constructors, member access, and `Result` inspection (`ResultIsOk`/`ResultValue`/`ResultError`) are well-typed.
* Every binding is defined before use; no use-after-move.
* Every path through the body produces a `Result` consistent with the declared success type — declared return/effect agreement.
* The source-level rule that `PROPAGATE` appears only inside a `TRAP` region is enforced during compilation; `PROPAGATE` is lowered to a `Fail` op before serialization, so decoded IR contains no separate propagate node.
* `CallResult`/`ResultValue`/`ResultError` apply only to fallible (`Result`) expressions, on the structurally correct branch.
* `MATCH` is exhaustive (covers every value or has an `ELSE`).
* There is at most one function-level bottom `TRAP`; error routing is via the structured `Trap`/`Fail` ops, never via unwinding or arbitrary jumps.
* Calls pass the correct number and type of arguments.
* Isolated function restrictions are preserved.

Because control flow is structured (nested regions with explicit ends), there are no branch targets to validate and no "jump into a trap or cleanup region" to reject.

## Resource verifier

The resource verifier checks:

* Resource values are never copied.
* Resource values are not compared, printed, serialized, or stored in ordinary collections.
* Resource values are not captured by lambdas.
* Resource values are not sent to threads unless explicitly marked sendable.
* A resource is not used after close.
* A resource is not used after move.
* A resource is closed exactly once, by explicit close or by lexical drop at scope exit.
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

This directly addresses the `.mfp` verifier gap identified in the review: type-correct IR, define-before-use, resource ownership, structured control flow, package signature validation, and native-link manifest validation. 

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
  binaryReprLength = N

packageBinaryRepr
  BinaryReprHeader
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
    function 0: add(Integer, Integer) AS Integer  (zero-length code region)
  IR
    "MFBR" + version + IrProject { function 0 body: Return(Binary{ Add, Ident a, Ident b }) }
```

The function body is the structured Binary Representation node for `add`, which decodes back to:

```text
RETURN  ( a + b )
```

i.e. an `IrOp::Return` whose value is an `IrValue::Binary { op: Add, left: a, right: b }`. There are no registers or opcodes — the consumer decodes this back to IR and lowers it through `IR → NIR → native`. If `a + b` overflows at runtime, the checked `Add` produces `ErrOverflow` (`77050010`) and the function returns `Err` (or routes to a `TRAP` region if one encloses it).

---

# Pasteable short spec addition

This is the compact version I would add to your current `Build Artifacts` section:

````markdown
### `.mfp` Container Format

A `.mfp` package is a signed binary container followed by architecture-independent MFB binary representation.

All integers are little-endian. All strings are UTF-8 byte strings with a `u32` byte length. No strings are NUL-terminated.

The container header is:

```text
magic              8 bytes
containerMajor     u16
containerMinor     u16
binaryReprMajor      u16
binaryReprMinor      u16
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

binaryReprLength     u64

packageBinaryRepr    byte[binaryReprLength]
````

The package content hash and package signature use the entire `.mfp` file with only the `signature` byte range replaced by zero bytes of the same length:

```text
signatureStart = 26
signatureEnd   = signatureStart + signatureLength
coveredBytes   = file[0 : signatureStart] || zero[signatureLength] || file[signatureEnd : end]
contentHash    = SHA-256(coveredBytes)
signatureInput = "MFP-PACKAGE-v1" || contentHash || ident || version
```

This covers the magic, container version, binary representation version, flags, signature type, signature length, header metadata, binary representation length, and binary representation. It excludes only the actual signature bytes.

`signatureType = 0` means unsigned and requires `signatureLength = 0`. `signatureType = 1` means Ed25519 and requires `signatureLength = 64`. Unknown signature types reject the package. Public registry packages must use `signatureType = 1`; installs reject unsigned packages except for explicit `allowUnsignedLocal` exceptions on `path:` or `file:` sources.

The binary representation payload must contain a signed package manifest. The manifest package name, ident, version, identKey, identFingerprint, and signingFingerprint must match the header package name, ident, version, identKey, identFingerprint, and signingFingerprint.

### Package Binary Representation

The package binary representation begins with:

```text
bcMagic        4 bytes = "MFPC"
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

The binary representation container is at MFPC major version `2` (the structured Binary Representation; the old flat opcode payload was major `1` and is rejected).

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
IR
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

The binary representation is **structured Binary Representation**: a faithful, versioned serialization of the compiler's IR. It contains no machine code, native addresses, host pointers, platform-specific object layouts, opcodes, registers, or jumps. Control flow is nested (regions with explicit ends) and expressions are trees. Function bodies live in the `IR` section (id `16`, payload prefixed `"MFBR"` + `u16` version); the `FUNCTION_TABLE` describes functions and records zero-length code regions. Constants, strings, types, imports, exports, globals, functions, native bindings, and resources are referenced from the IR by table indexes.

Every function returns `Result` at the IR level. Source-level auto-unwrapping, inline `TRAP`, and direct `MATCH` on a call are all encoded as ordinary IR nodes (`CallResult`, `ResultIsOk`/`ResultValue`/`ResultError`, `Trap`, `Match`). A consumer decodes the Binary Representation back to IR, applies the package identity prefix, merges it into the project, and lowers everything through the single `IR → NIR → native` path.

The verifier checks the decoded IR: section bounds, type references, type-correctness, define-before-use, resource ownership/linearity, exhaustive `MATCH`, single bottom trap, declared return/effect agreement, native binding metadata, and package signature validity before the package may be imported or merged.

```

That gives you a concrete `.mfp` container and a structured Binary Representation payload that rejoins the single native codegen, without a separate package VM or a second binary representation→native bridge.
```

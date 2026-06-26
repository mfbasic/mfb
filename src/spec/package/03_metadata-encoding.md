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

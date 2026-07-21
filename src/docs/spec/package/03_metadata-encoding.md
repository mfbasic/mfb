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

The manifest repeats the container header identity: `packageName`,
`packageIdent`, `packageVersion`, and `identKey` must equal the header's
`name`, `ident`, `version`, and `identKey`, and the manifest
`identFingerprint`/`signingFingerprint` must equal the SHA-256 fingerprints
**derived from** the header's full `identKey`/`signingKey` (the container
header no longer carries fingerprint fields). For an unsigned package all five
identity-chain strings are empty. A signed build stamps these fields from the
signing bundle (`apply_signing_metadata`); they are never read from the
project manifest. [[src/binary_repr/reader.rs:validate_container_manifest_identity]][[src/cli/build.rs:apply_signing_metadata]]

Current compiler values (`encode_manifest`):

* `binaryReprMajor`/`binaryReprMinor` are written as `1`/`0`, `languageMajor`/`languageMinor` as `1`/`0`, and `minimumRuntimeMajor`/`minimumRuntimeMinor` as `1`/`0`. These are emitted as fixed constants, not derived, and the reader (`read_manifest`) reads past them without validating. [[src/binary_repr/writer.rs:encode_manifest]]
* `dependencyCount` equals the number of `IMPORT_TABLE` entries.
* `nativeLinkCount` is **always `0`** — native binding counts are not surfaced here (native `LINK` data lives in the `IR` payload trailer). The reader reads and discards it.
* `exportCount` equals the number of exported callable functions (see `EXPORT_TABLE`).

`entryFunction` identifies the executable entry point when the binary representation payload is the root executable payload or has been produced by merging package binary representation into the root project binary representation. Reusable packages set it to `0xFFFFFFFF` and `entryFlags` to `0`. Entry flags:

```text
bit 0 = package has executable entry
bit 1 = entry accepts command-line args as List OF String
bit 2 = entry is FUNC returning Integer
```

How the runtime maps an entry-point outcome to a process exit code and stderr (the `SUB`/`FUNC ... AS Integer`/uncaught-error contract) is the source-level entry-point contract, owned by `./mfb spec language error-model`. When args are accepted, argument element zero is the program name as invoked by the host.

When reading a **package** (as opposed to a root executable image), the current reader ignores the manifest's `entryFunction`/`entryFlags` and forces them to `0xFFFFFFFF`/`0` in the decoded project, since a reusable package has no entry point of its own.

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

The current compiler always writes `flags = 0` — manifest dependency lowering never sets bit 0 today, so it is effectively reserved. The reader reads the field without acting on it. [[src/manifest/package.rs:package_dependencies]]

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

Export kinds the current compiler emits and the reader accepts:

```text
1 = function   (exported FUNC)
2 = sub        (exported SUB)
```

The current `EXPORT_TABLE` carries **only callable exports** — exported `FUNC` (kind `1`) and `SUB` (kind `2`). `encode_exports` walks the function table and writes one entry per exported (non-private) function; `targetId` is that function's index in `FUNCTION_TABLE`, `flags` is `0`. The reader (`read_export_table` → `decode_callable_export_kind`) accepts only kinds `1` and `2` and rejects any other value. [[src/binary_repr/reader.rs:decode_callable_export_kind]]

Exported **types** (record/union/enum) are not listed in `EXPORT_TABLE`. They are surfaced through `TYPE_TABLE` and `ABI_INDEX` instead (the latter carrying their ABI hashes). Higher kind numbers for top-level `LET`/`MUT`, constructors, and native wrappers are not part of the current encoding.

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

`abiFormatVersion = 1` uses SHA-256 hashes (`read_abi_index` rejects any other version). Each exported ABI entry is `name` (stringId), `kind` (u16, using the **export-kind numbering**, not a separate ABI numbering), and a 32-byte `abiHash`.

The current declaration kinds in `ABI_INDEX` are exactly the kinds `encode_export_kind` produces:

```text
1 = exported FUNC
2 = exported SUB
3 = exported record type
4 = exported union type
5 = exported enum type
```

The ABI index emits one entry per exported function (kinds `1`/`2`) followed by one entry per exported type whose ABI export kind is set (kinds `3`/`4`/`5`). [[src/binary_repr/sections.rs:from_project]] Exported constants, globals, native wrappers, and resource types are **not** currently given their own ABI entries — the kinds `6`-`10` are not produced. (A resource type does appear, but as its underlying record type, kind `3`.)

Because `ABI_INDEX` lives inside `packageBinaryRepr`, the `packageBinaryHash` and the package signature already cover it — no header change is needed to trust it. The registry parses this section from a published package (string pool + `ABI_INDEX` only) and serves the resulting `{ "<symbol>": "<hex abiHash>" }` map as the `abiIndex` field of `GET /index`. `mfb repo check-abi` builds the working tree, reads its `ABI_INDEX`, and diffs it against the latest published version's served map, naming every changed or dropped symbol (both break the superset relation the resolver relies on).[[repository/src/abi.rs:parse_abi_index]][[src/cli/pkg.rs:check_abi]]

The hash input is built by `AbiSerializer` and begins with `MFBABI\0` followed by `abiFormatVersion` (u16, currently `1`; it gates the ABI_INDEX *wire encoding*, so a change to what the hash is computed over does not bump it — a stale hash is caught per symbol by `validate_abi_index` instead). For a **function or sub** (`function_sig_hash`) the remaining input is: [[src/binary_repr/reader.rs:function_sig_hash]]

* the literal string `"function"`,
* the export kind (u16),
* the function flags **masked to `ISOLATED | SUB`** (u16) — only the isolated and sub bits are ABI-significant; other flags are excluded,
* the parameter count (u32),
* for each parameter, its structurally-serialized type, then a default-presence byte (`0`/`1`) and, when present, the serialized default constant,
* the structurally-serialized return type.

Note what is **not** in the function hash today: parameter names, resource ownership/non-owning/consume annotations, and explicit error/result behaviour. Two functions that differ only in those respects currently hash identically.

For an exported **type** (`type_sig_hash`) the input is `MFBABI\0`, `abiFormatVersion`, the literal `"type"`, the export kind (u16), and the structural serialization of the type. [[src/binary_repr/reader.rs:type_sig_hash]] The structural serializer (`serialize_type`) encodes records as their field names + field types + visibility, unions as their variant names + variant fields, enums as their member names + ordinals, and the compiler-owned templates `List`/`Map`/`Result`/`Thread` and function types by a tag string (`"list"`/`"map"`/`"result"`/`"thread"`/`"function-type"`) plus their structurally-serialized component types. A resource carrying a `STATE` payload (kind `11`) is likewise serialized structurally, by the tag `"state"` plus its base type and its state type. `MapEntry` (kind `9`) and `ThreadWorker` (kind `10`) are **not** serialized structurally: they fall through to the `"opaque"` arm and are hashed by their canonical interned name (e.g. `ThreadWorker#<msgTypeId>#<outTypeId>`); because that name embeds this package's numeric table type ids, the resulting hash is sensitive to type-table id assignment, not only to the type's shape. Primitive types serialize by id + name; a back-reference scheme keeps recursive/shared types finite. [[src/binary_repr/reader.rs:serialize_type]] Reordering record fields, union variants, or enum members — or changing an ordinal — therefore changes the hash.

`exportAbiCount` is validated against `EXPORT_TABLE` by `validate_abi_index`: every `EXPORT_TABLE` entry must have a matching `ABI_INDEX` entry with the same `name` and `kind` **and** a `sigHash` equal to the hash recomputed from the function table. [[src/binary_repr/reader.rs:validate_abi_index]] Because `EXPORT_TABLE` holds only callable exports while `ABI_INDEX` additionally holds type exports, the two are **not** required to have equal length or matching order — only that each callable export is covered.

`dependencyAbiCount` is validated against `IMPORT_TABLE` by package import name and package ident (`validate_abi_index` requires the sorted `(name, ident)` sets to match). Each dependency ABI entry repeats the requested `version` and `pin` state and records every imported symbol whose ABI shape was used while compiling this package, and the reader requires the dependency edge's `version`/`pin`/used-symbol list to match the corresponding `IMPORT_TABLE` entry exactly. These hashes are also present in `IMPORT_TABLE` so tools that only need dependency requirements can read one section.

A future ABI index version may add the missing declaration kinds (constants, globals, native wrappers, standalone resource entries) and richer per-declaration hashes; v1 as implemented covers callable functions and the three user type kinds.

## See Also

* ./mfb spec package binary-representation — the section framing these metadata tables live in
* ./mfb spec package type-table — a metadata table referencing the string pool
* ./mfb spec package constant-pool — another indexed metadata table

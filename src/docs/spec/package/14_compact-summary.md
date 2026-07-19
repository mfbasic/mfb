# Compact summary

This page is a deliberately redundant quick-reference: a compact, single-page restatement of the `.mfp` container and structured Binary Representation payload for readers who want the whole format at a glance. The canonical per-section detail lives in the other `package` topics.

### `.mfp` Container Format

A `.mfp` package is a signed binary container followed by architecture-independent MFB binary representation.

All integers are little-endian. All strings are UTF-8 byte strings with a `u32` byte length. No strings are NUL-terminated.

The container is **hard version 1.0** (`containerMajor = 1`, `containerMinor
= 0`, verified exactly). The header is:

```text
magic              8 bytes
containerMajor     u16   = 1
containerMinor     u16   = 0
binaryReprMajor    u16
binaryReprMinor    u16
flags              u32

nameLength         u32
name               byte[nameLength]

identLength        u32
ident              byte[identLength]

versionLength      u32
version            byte[versionLength]

authorLength       u32
author             byte[authorLength]

urlLength          u32
url                byte[urlLength]

identKeyLength     u32
identKey           byte[identKeyLength]         ident PUBLIC key

signingKeyLength   u32
signingKey         byte[signingKeyLength]       one-off PUBLIC key

proofLength        u32
proof              byte[proofLength]            JSON, ident-signed

proofSigLength     u32
proofSig           byte[proofSigLength]         64-byte ident signature

attestationLength  u32
attestation        byte[attestationLength]      JSON, server-signed

attestationSigLength u32
attestationSig     byte[attestationSigLength]   64-byte server signature

packageBinaryHash  byte[32]                     SHA-256 of packageBinaryRepr

binaryReprLength   u64

signatureType      u16
signatureLength    u32
signature          byte[signatureLength]        by the one-off signing key

packageBinaryRepr  byte[binaryReprLength]
```

The signature is a **prefix signature** by the one-off signing key
(`signingKey`): it covers every byte before the signature itself, and the
payload transitively through `packageBinaryHash`:

```text
signedPrefix   = file[0 : offset of signature]
signatureInput = "MFP-PACKAGE-v2\0" || SHA-256(signedPrefix)
contentHash    = SHA-256(entire file)        (blob/dedup identity)
```

`signatureType = 0` means unsigned and requires `signatureLength = 0` and every trust-chain field (`identKey`, `signingKey`, `proof`, `proofSig`, `attestation`, `attestationSig`) empty. `signatureType = 1` means Ed25519 and requires `signatureLength = 64` and every trust-chain field present. Unknown signature types reject the package. Whether an unsigned package is *accepted* is package-manager policy (local `file://` dependencies only), not part of this byte format (see `./mfb spec architecture packages`).

The binary representation payload must contain a signed package manifest. The manifest package name, ident, version, and identKey must match the header, and the manifest ident/signing fingerprints must equal the SHA-256 fingerprints derived from the header `identKey`/`signingKey`.

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

Sections the reader requires (rejecting the package if absent) are:

```text
MANIFEST
STRING_POOL
TYPE_TABLE
CONST_POOL
IMPORT_TABLE
EXPORT_TABLE
FUNCTION_TABLE
IR
ABI_INDEX
```

The reader treats `NATIVE_LIBRARY_TABLE` (id 10), `GLOBAL_TABLE` (id 7), `RESOURCE_TABLE` (id 11), and `DOC` (id 17) as optional, defaulting them to empty when absent. [[src/binary_repr/reader.rs:read_binary_repr_package]] The producer always emits `GLOBAL_TABLE`, and emits `RESOURCE_TABLE`/`DOC`/`NATIVE_LIBRARY_TABLE` only when the package has resource types, documentation, or a `LINK` block respectively. Section ids `12` (`DEBUG_INFO`), `13` (`SOURCE_MAP`), and `14` (`AUDIT_INFO`) are reserved by the format but **not** emitted or read by the current compiler.

Native `LINK` metadata is split across both: the per-function **interface** rides as a trailer inside the `IR` payload, while the per-library **locators** (which shared object to load per `os`/`arch`/`libc`) live in `NATIVE_LIBRARY_TABLE` (id 10), whose presence also sets container flag bit 0. See `native-bindings`.

The binary representation is **structured Binary Representation**: a faithful, versioned serialization of the compiler's IR. It contains no machine code, native addresses, host pointers, platform-specific object layouts, opcodes, registers, or jumps. Control flow is nested (regions with explicit ends) and expressions are trees. Function bodies live in the `IR` section (id `16`, payload prefixed `"MFBR"` + `u16` version); the `FUNCTION_TABLE` describes functions and records zero-length code regions. The `MFBR`/IR payload carries every name and type **inline** and round-trips to IR standalone; the metadata tables (constants, strings, types, imports, exports, globals, functions, native bindings, resources) are a parallel, derived view used for scanning, ABI checks, and identity — the IR does not reference them by index. [[src/ir/binary.rs:encode_project]]

Every function returns `Result` at the IR level. Source-level auto-unwrapping, inline `TRAP`, and direct `MATCH` on a call are all encoded as ordinary IR nodes (`CallResult`, `ResultIsOk`/`ResultValue`/`ResultError`, `Trap`, `Match`). A consumer decodes the Binary Representation back to IR, applies the package identity prefix, merges it into the project, and lowers everything through the single `IR → NIR → native` path.

At **import time** the reader checks: container magic/version/identity, MFPC `bcMajor == 2`, section bounds, presence of required sections, exact table parsing, and `ABI_INDEX` agreement with `EXPORT_TABLE`/`IMPORT_TABLE`. Type-correctness, define-before-use, resource ownership/linearity, exhaustive `MATCH`, and declared return/effect agreement are not re-checked at *import/read* time, but they **are** re-established at *merge* time: the complete semantic verifier — the same one used on the project's own source-lowered IR — runs over the merged package IR before native lowering (see verifier-rules). [[src/ir/verify/mod.rs:check]] The cryptographic signature is verified by the package manager, not the binary-representation reader.

The result is a concrete `.mfp` container and a structured Binary Representation payload that rejoins the single native codegen, without a separate package VM or a second binary-representation→native bridge.

## See Also

* ./mfb spec package container-format — the canonical `.mfp` header and signature coverage
* ./mfb spec package binary-representation — the canonical MFPC payload and section table

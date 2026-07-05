# Compact summary

This page is a deliberately redundant quick-reference: a compact, single-page restatement of the `.mfp` container and structured Binary Representation payload for readers who want the whole format at a glance. The canonical per-section detail lives in the other `package` topics.

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
```

The package content hash and package signature use the entire `.mfp` file with only the `signature` byte range replaced by zero bytes of the same length:

```text
signatureStart = 26
signatureEnd   = signatureStart + signatureLength
coveredBytes   = file[0 : signatureStart] || zero[signatureLength] || file[signatureEnd : end]
contentHash    = SHA-256(coveredBytes)
signatureInput = "MFP-PACKAGE-v1" || contentHash || ident || version
```

This covers the magic, container version, binary representation version, flags, signature type, signature length, header metadata, binary representation length, and binary representation. It excludes only the actual signature bytes.

`signatureType = 0` means unsigned and requires `signatureLength = 0`. `signatureType = 1` means Ed25519 and requires `signatureLength = 64`. Unknown signature types reject the package. Whether an unsigned or untrusted package is *accepted* is package-manager policy, not part of this byte format (see `./mfb spec architecture packages`).

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

The reader treats `GLOBAL_TABLE` (id 7), `RESOURCE_TABLE` (id 11), and `DOC` (id 17) as optional, defaulting them to empty when absent. [[src/binary_repr/reader.rs:read_binary_repr_package]] The producer always emits `GLOBAL_TABLE`, and emits `RESOURCE_TABLE`/`DOC` only when the package has resource types or documentation respectively. Section ids `10` (`NATIVE_LINK_TABLE`), `12` (`DEBUG_INFO`), `13` (`SOURCE_MAP`), and `14` (`AUDIT_INFO`) are reserved by the format but **not** emitted or read by the current compiler. Native `LINK` metadata is carried as a trailer inside the `IR` payload, not in a `NATIVE_LINK_TABLE` section.

The binary representation is **structured Binary Representation**: a faithful, versioned serialization of the compiler's IR. It contains no machine code, native addresses, host pointers, platform-specific object layouts, opcodes, registers, or jumps. Control flow is nested (regions with explicit ends) and expressions are trees. Function bodies live in the `IR` section (id `16`, payload prefixed `"MFBR"` + `u16` version); the `FUNCTION_TABLE` describes functions and records zero-length code regions. The `MFBR`/IR payload carries every name and type **inline** and round-trips to IR standalone; the metadata tables (constants, strings, types, imports, exports, globals, functions, native bindings, resources) are a parallel, derived view used for scanning, ABI checks, and identity — the IR does not reference them by index. [[src/ir/binary.rs:encode_project]]

Every function returns `Result` at the IR level. Source-level auto-unwrapping, inline `TRAP`, and direct `MATCH` on a call are all encoded as ordinary IR nodes (`CallResult`, `ResultIsOk`/`ResultValue`/`ResultError`, `Trap`, `Match`). A consumer decodes the Binary Representation back to IR, applies the package identity prefix, merges it into the project, and lowers everything through the single `IR → NIR → native` path.

At **import time** the reader checks: container magic/version/identity, MFPC `bcMajor == 2`, section bounds, presence of required sections, exact table parsing, and `ABI_INDEX` agreement with `EXPORT_TABLE`/`IMPORT_TABLE`. Type-correctness, define-before-use, resource ownership/linearity, exhaustive `MATCH`, and declared return/effect agreement are not re-checked at *import/read* time, but they **are** re-established at *merge* time: `ir::verify_semantics` — the complete semantic checker, the same one used on the project's own source-lowered IR — runs over the merged package IR before native lowering (see verifier-rules). The cryptographic signature is verified by the package manager, not the binary-representation reader.

The result is a concrete `.mfp` container and a structured Binary Representation payload that rejoins the single native codegen, without a separate package VM or a second binary-representation→native bridge.

## See Also

* ./mfb spec package container-format — the canonical `.mfp` header and signature coverage
* ./mfb spec package binary-representation — the canonical MFPC payload and section table

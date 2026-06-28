# Container Format

The `.mfp` container wraps the package binary representation with a signed header that carries package identity, signature, and the metadata a package manager needs to scan files without parsing every table.

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
| `binaryReprMajor`   | Major version of the package Binary Representation format. The current compiler writes `1` here and the reader **does not validate this field** (see note below). |
| `binaryReprMinor`   | Minor version of the package Binary Representation format. The current compiler writes `0`. |
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

### Two distinct version numbers

There are two independent version numbers, and the current compiler gives them different values:

* The container header `binaryReprMajor`/`binaryReprMinor` fields above carry `1`/`0`. The current reader (`mfp_binary_repr_payload` in `src/binary_repr.rs`, and the `MfpHeader` reader in `src/main.rs`) reads past these fields without validating them.
* The **MFPC payload** header inside `packageBinaryRepr` carries its own `bcMajor`, which is `2` (the clean break to the structured Binary Representation; see `binary-representation`). The reader validates **this** value, rejecting any payload whose `bcMajor` is not `2`.

In other words, the "version 2" clean break lives in the MFPC payload, not in the container header field of the same name. Implementers should not conflate the two.

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

These on-disk encoding rules are all the `binary_repr` reader enforces. Whether an unsigned or untrusted package is *accepted* — registry signing requirements, `mfb pkg install` defaults, `allowUnsignedLocal` exceptions, `mfb.lock` recording, and per-ident/key trust policy — is package-manager policy, not part of this byte format. See `./mfb spec architecture packages`.

## Container flags

```text
bit 0 = package contains native LINK metadata   (reserved; not currently emitted)
bit 1 = package contains debug metadata          (reserved; not currently emitted)
bit 2 = package contains source-map metadata     (reserved; not currently emitted)
bit 3 = package is pre-release
bits 4-15 = reserved optional flags
bits 16-31 = reserved required flags
```

Current compiler behaviour (`container_flags` in `src/target/package_mfp/mod.rs`): the only flag the compiler ever sets is **bit 3 (pre-release)**, and it sets it exactly when the package `version` string contains a `-` (a semantic-version pre-release tag). [[src/target/package_mfp/mod.rs:container_flags]] Bits 0-2 are defined by the format but are **not currently emitted** — native LINK metadata is carried inside the binary representation payload rather than signalled by a container flag (see `native-bindings`), and debug/source-map metadata are not produced. The current reader does not act on the flags field.

The reserved-required-flag rule remains normative for forward compatibility: if an implementation sees an unknown required flag (bits 16-31), it must reject the package before import or merge.

Current compiler source of truth:

- Package/container rejection currently comes from detailed package-reader diagnostics in `src/binary_repr.rs`, `src/target/package_mfp/mod.rs`, and `src/main.rs`.
- These failures are currently surfaced as descriptive `error: ...` strings such as invalid magic, invalid signature header, truncated signature, or unsupported binary representation/container version rather than through a single package rule code path.

## Container validation

The current container reader (`mfp_binary_repr_payload` in `src/binary_repr.rs`, mirrored by the `MfpHeader` reader in `src/main.rs`) rejects an `.mfp` package when:

* The file is shorter than the 26-byte fixed prefix. The current compiler reports this as `package is too small to be a valid .mfp package`.
* `magic` does not match. The current compiler reports this as `package does not have the MFP package magic`.
* `containerMajor` is not `1`. The current compiler reports this as `unsupported MFP container major version <n>`.
* `signatureType` is unknown. The current compiler reports this as `unsupported .mfp signature type <n>`.
* `signatureLength` is invalid for the signature type. The current compiler reports either `unsigned .mfp package must have zero signature length` or `Ed25519 .mfp package must have a 64 byte signature`. [[src/binary_repr/reader.rs:validate_mfp_signature_header]]
* The declared signature length runs past the end of the file. The current compiler reports this as `truncated .mfp signature`.
* `binaryReprLength` does not exactly match the remaining byte count, or there are trailing bytes after `packageBinaryRepr`. The current compiler reports both as `invalid .mfp binary representation length`.
* The container header identity does not match the embedded binary representation manifest identity. The current compiler reports this as `MFP header identity does not match binary representation manifest identity`. The identity comparison covers `name`, `ident`, `version`, `identKey`, `identFingerprint`, and `signingFingerprint` (`validate_container_manifest_identity`). [[src/binary_repr/reader.rs:validate_container_manifest_identity]]

The MFPC payload's own `bcMajor` (which must be `2`) is checked separately when the payload is parsed (`read_binary_repr_package`), reported as `unsupported MFPC major version <n> (expected 2); this package predates the structured Binary Representation format and must be rebuilt`. This is a **clean break** from the old flat opcode payload (`bcMajor = 1`), which is rejected outright.

What the container reader does **not** do:

* It does **not** verify the cryptographic signature. Signature/trust-policy verification is performed by the package manager layer (`mfb_repository::crypto`) at install/resolve time, not by the binary-representation reader at import time. `package_content_hash` and `build_signed_package_bytes` in `src/target/package_mfp/mod.rs` produce and cover the signature; the import-time reader treats the signature bytes only as a region to skip over. [[src/target/package_mfp/mod.rs:package_content_hash]]
* It does **not** validate the container header `binaryReprMajor`/`binaryReprMinor` fields.

The `MfpHeader` reader in `src/main.rs` additionally enforces the recommended string-length limits below while reading the header strings; the binary-representation reader path does not re-check them.

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

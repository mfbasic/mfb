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

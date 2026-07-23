# Container Format

The `.mfp` container wraps the package binary representation with a header that
carries package identity, the trust chain (ident key, one-off signing
key, ident-signed proof, server-signed attestation), a payload hash that welds
the header to the payload, and a prefix signature over every header byte.

The container is **hard version 1.0**: readers verify `containerMajor = 1` and
`containerMinor = 0` exactly, with no backwards compatibility for earlier
layouts. Packages produced by older writers must be rebuilt.

## Container layout

```text
.mfp file
  MFPHeader        (signed prefix, then the signature)
  packageBinaryRepr
```

## `MFPHeader`

```text
magic              8 bytes
containerMajor     u16      = 1
containerMinor     u16      = 0
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
identKey           byte[identKeyLength]        ident PUBLIC key

signingKeyLength   u32
signingKey         byte[signingKeyLength]      one-off PUBLIC key

proofLength        u32
proof              byte[proofLength]           JSON, ident-signed at build time

proofSigLength     u32
proofSig           byte[proofSigLength]        64-byte ident signature

attestationLength  u32
attestation        byte[attestationLength]     JSON, server-signed per build

attestationSigLength u32
attestationSig     byte[attestationSigLength]  64-byte server signature

packageBinaryHash  byte[32]                    SHA-256 of packageBinaryRepr

binaryReprLength   u64

signatureType      u16
signatureLength    u32
signature          byte[signatureLength]       made by the one-off signing key;
                                               signs everything above this point

packageBinaryRepr  byte[binaryReprLength]
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
| `containerMajor`  | Major version of the `.mfp` container format. Must be `1`.        |
| `containerMinor`  | Minor version of the `.mfp` container format. Must be `0`.        |
| `binaryReprMajor` | Major version of the package Binary Representation format. The current compiler writes `1` here and the reader **does not validate this field** (see note below). |
| `binaryReprMinor` | Minor version of the package Binary Representation format. The current compiler writes `0`. |
| `flags`           | Container-level flags. Unknown required flags reject the package. |
| `name`            | Source import name, such as `"sqlite"` or `"geometry"`.           |
| `ident`           | Registry identity `<owner>#<package>` for resolved packages.      |
| `version`         | Package version string.                                           |
| `author`          | Informational author string.                                      |
| `url`             | Informational package/project URL.                                |
| `identKey`        | The owner's ident public key, metadata form `ed25519:<base64url>`. Empty when unsigned. |
| `signingKey`      | The one-off per-package signing public key, metadata form. Empty when unsigned. |
| `proof`           | Proof JSON (see *package-manager signing*): ident-signed statement pinning this exact `ident`, `version`, and both key fingerprints. Empty when unsigned. |
| `proofSig`        | 64-byte Ed25519 signature over `"MFP-PROOF-v1\0" \|\| proof` by the ident key. Empty when unsigned. |
| `attestation`     | Attestation JSON: registry-signed statement pinning the same fields plus the registry fingerprint. Empty when unsigned. |
| `attestationSig`  | 64-byte Ed25519 signature over `"MFP-ATTEST-v1\0" \|\| attestation` by the registry server key. Empty when unsigned. |
| `packageBinaryHash` | Raw 32-byte SHA-256 of `packageBinaryRepr`. Welds the payload to the signed header. |
| `binaryReprLength`  | Exact byte length of `packageBinaryRepr`.                       |
| `signatureType`   | Signature algorithm identifier.                                   |
| `signatureLength` | Number of bytes in `signature`.                                   |
| `signature`       | Package signature bytes, made by the **one-off signing key**.     |
| `packageBinaryRepr` | Architecture-independent MFB Binary Representation image.       |

The header `name`, `ident`, `version`, `identKey`, `author`, and `url` are for
fast package scanning. The binary representation payload must also contain a
signed manifest that repeats the header identity: the same `name`, `ident`,
`version`, and `identKey`, plus the SHA-256 fingerprints of the header's
`identKey` and `signingKey` (the fingerprints are derived from the full keys;
they are no longer header fields). A verifier must reject the package if the
header identity and binary representation manifest identity do not match.
[[src/binary_repr/reader.rs:validate_container_manifest_identity]]

### Two distinct version numbers

There are two independent version numbers, and the current compiler gives them different values:

* The container header `binaryReprMajor`/`binaryReprMinor` fields above carry `1`/`0`. The current reader reads past these fields without validating them. [[src/binary_repr/reader.rs:mfp_binary_repr_payload]] [[src/manifest/package.rs:read_mfp_header]]
* The **MFPC payload** header inside `packageBinaryRepr` carries its own `bcMajor`, which is `2` (the clean break to the structured Binary Representation; see `binary-representation`). The reader validates **this** value, rejecting any payload whose `bcMajor` is not `2`.

In other words, the "version 2" clean break lives in the MFPC payload, not in the container header field of the same name. Implementers should not conflate the two.

## Signature coverage

The signature is a **prefix signature**: it covers every byte of the file
before the signature bytes themselves — from `magic` through
`signatureType`/`signatureLength` inclusive, including `packageBinaryHash` and
`binaryReprLength`. The payload is covered transitively through
`packageBinaryHash`, so the header is welded to the payload, header grafting is
impossible, and the payload can be streamed and verified separately.

```text
signedPrefix = file[0 : offset of signature]

signature input ("MFP-PACKAGE-v2" is ASCII; \0 is one NUL byte):

    "MFP-PACKAGE-v2\0" || SHA-256(signedPrefix)
```

The signature is made by the **one-off signing key** whose public half is the
header `signingKey` (see *package-manager signing* for the key model). The
domain tag prevents a package signature from being replayed as a proof,
attestation, or any other Ed25519 signature in the system.
[[repository/src/crypto.rs:package_signing_input]][[src/target/package_mfp/mod.rs:build_package_bytes]]

Verification must use the raw byte sequence exactly as stored. There is no
string normalization, metadata canonicalization, JSON normalization, or
re-serialization before verification.

Proof and attestation signatures are domain-tagged the same way:
`"MFP-PROOF-v1\0" || proofBytes` (ident key) and
`"MFP-ATTEST-v1\0" || attestationBytes` (registry server key).
[[repository/src/crypto.rs:proof_signing_input]]

### Content hash

The whole-file SHA-256 is the package's blob/dedup identity in the publish
flow. Because the prefix signature covers the header and `packageBinaryHash`
covers the payload, a signed file is immutable after signing and the content
hash needs no signature-zeroing construction.
[[src/target/package_mfp/mod.rs:package_content_hash]]

## Signature types

```text
0 = unsigned
1 = Ed25519
```

Rules:

* `signatureType = 0` means the package is unsigned.
* If `signatureType = 0`, then `signatureLength` must be `0`, and the trust
  chain fields (`identKey`, `signingKey`, `proof`, `proofSig`, `attestation`,
  `attestationSig`) must all be empty — an unsigned package carries no
  identity chain.
* `signatureType = 1` means Ed25519.
* If `signatureType = 1`, then `signatureLength` must be `64`, and every trust
  chain field must be present (non-empty). A partial chain is malformed.
* Unknown `signatureType` values reject the package.

Unsigned packages are permitted for local `file://` development dependencies
only; registry publishes always require the full signed chain. Whether an
unsigned package is *accepted* is package-manager policy (see the `--unsigned`
build gate and `./mfb spec architecture packages`); the byte-format rules above
are enforced by every reader. [[repository/src/package.rs:parse_mfp_package]]

## Container flags

```text
bit 0 = package contains native LINK metadata   (set when the package carries a NATIVE_LIBRARY_TABLE)
bit 1 = package contains debug metadata          (reserved; not currently emitted)
bit 2 = package contains source-map metadata     (reserved; not currently emitted)
bit 3 = package is pre-release
bits 4-15 = reserved optional flags
bits 16-31 = reserved required flags
```

Current compiler behaviour: the compiler sets **bit 3 (pre-release)** exactly when the package `version` string contains a `-` (a semantic-version pre-release tag), and **bit 0 (native LINK metadata)** exactly when the package carries a `NATIVE_LIBRARY_TABLE` (section id 10) — i.e. for a binding package that declares a `LINK` block (see `native-bindings`). [[src/target/package_mfp/mod.rs:container_flags]] Bits 1-2 are defined by the format but are **not currently emitted**: debug and source-map metadata are not produced. The current reader does not act on the flags field.

Bit 0 is an **optional** flag: section 10 is the source of truth, so a reader that ignores the bit must not reject the package.

The reserved-required-flag rule remains normative for forward compatibility: if an implementation sees an unknown required flag (bits 16-31), it must reject the package before import or merge.

Current compiler behaviour:

- Package/container rejection currently comes from detailed package-reader diagnostics. [[src/binary_repr/reader.rs]] [[src/target/package_mfp/mod.rs]] [[src/manifest/package.rs]] [[repository/src/package.rs]]
- These failures are currently surfaced as descriptive `error: ...` strings such as invalid magic, invalid signature header, truncated signature, or unsupported binary representation/container version rather than through a single package rule code path.

## Container validation

The current container readers reject an `.mfp` package when: [[src/binary_repr/reader.rs:mfp_binary_repr_payload]] [[src/manifest/package.rs:read_mfp_header]] [[repository/src/package.rs:parse_mfp_package]]

* The file is shorter than the 20-byte fixed prefix. The current compiler reports this as `package is too small to be a valid .mfp package`.
* `magic` does not match. The current compiler reports this as `package does not have the MFP package magic`.
* `containerMajor.containerMinor` is not exactly `1.0`. The current compiler reports this as `unsupported MFP container version <maj>.<min> (expected 1.0)`. This check is hard: there is no backwards compatibility with earlier layouts.
* Any length-prefixed field exceeds its limit or runs past the end of the file (`.mfp <field> exceeds the <limit> byte limit` / `truncated .mfp <field>`).
* `signatureType` is unknown. The current compiler reports this as `unsupported .mfp signature type <n>`.
* `signatureLength` is invalid for the signature type. The current compiler reports either `unsigned .mfp package must have zero signature length` or `Ed25519 .mfp package must have a 64 byte signature`. [[src/binary_repr/reader.rs:validate_mfp_signature_header]]
* The declared signature length runs past the end of the file. The current compiler reports this as `truncated .mfp signature`.
* `binaryReprLength` does not exactly match the remaining byte count, or there are trailing bytes after `packageBinaryRepr`. The current compiler reports both as `invalid .mfp binary representation length`.
* A signed package is missing any trust chain field, or an unsigned package carries one (`signed .mfp package is missing <field>` / `unsigned .mfp package must not carry <field>`). [[repository/src/package.rs:parse_mfp_package]]
* The container header identity does not match the embedded binary representation manifest identity. The current compiler reports this as `MFP header identity does not match binary representation manifest identity`. The identity comparison covers `name`, `ident`, `version`, `identKey`, and the fingerprints of the header `identKey`/`signingKey` against the manifest's recorded fingerprints. [[src/binary_repr/reader.rs:validate_container_manifest_identity]]

The MFPC payload's own `bcMajor` (which must be `2`) is checked separately when the payload is parsed, reported as `unsupported MFPC major version <n> (expected 2); this package predates the structured Binary Representation format and must be rebuilt`. [[src/binary_repr/reader.rs:read_binary_repr_package]] This is a **clean break** from the old flat opcode payload (`bcMajor = 1`), which is rejected outright.

What the import-time container reader does **not** do:

* It does **not** verify the cryptographic signature, proof, attestation, or `packageBinaryHash`. Trust verification is performed by the package-manager layer at build/install time — the trust verification chain (see `verifier-rules`) — not by the binary-representation reader at import time. The import-time reader treats the signature bytes only as a region to skip over. [[src/cli/build.rs:classify_installed_package]]
* It does **not** validate the container header `binaryReprMajor`/`binaryReprMinor` fields.

Recommended limits (enforced by the header and package readers while reading; `name`, `ident`, and `version` must be non-empty in the repository reader, and every string field must be valid UTF-8): [[src/manifest/package.rs:read_mfp_header]] [[repository/src/package.rs:parse_mfp_package]]

```text
nameLength                <= 255
identLength               <= 255
versionLength             <= 64
authorLength              <= 512
urlLength                 <= 2048
identKeyLength            <= 255
signingKeyLength          <= 255
proofLength               <= 4096
proofSigLength            <= 64
attestationLength         <= 4096
attestationSigLength      <= 64
binaryReprLength          <= implementation-defined maximum
```

Package names should use the same identifier restrictions as source package names unless the package manager later defines a wider registry naming scheme.

`name` is additionally constrained to a single safe path component — `[A-Za-z0-9_][A-Za-z0-9_.-]*` — because every consumer installs the package as `packages/<name>.mfp`. This is enforced both when a package is built and when its header is read, so a crafted `name` such as `../../x` cannot escape the project directory. [[src/manifest/package.rs:validate_package_name]] The same constraint applies to each **dependency** `name` recorded in the metadata's dependency list, since a consumer resolves those as `packages/<name>.mfp` too; a build refuses to produce a container carrying a traversing dependency name. [[src/target/package_mfp/mod.rs:validate_metadata]]

An installed package is written by staging the untrusted blob under an exclusively created name inside `packages/`, verifying it there, and only then renaming it onto `packages/<name>.mfp`. Nothing attacker-controlled is ever written to the final path before it verifies, and a symlink planted at that path is replaced by the rename rather than written through. [[src/cli/mod.rs:install_verified_package]]

## See Also

* ./mfb spec package binary-representation — the payload this container wraps
* ./mfb spec package metadata-encoding — the metadata tables inside the payload
* ./mfb spec tooling auditability — the signing and trust-chain model behind the header

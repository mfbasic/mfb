# p256Verify

Verify an ECDSA P-256/SHA-256 signature against a public key (FIPS 186).

## Synopsis

```
crypto::p256Verify(publicKey AS List OF Byte, message AS List OF Byte, signature AS List OF Byte) AS Boolean
```

## Package

crypto

## Imports

```
IMPORT crypto
```

`crypto` is a built-in package, so no manifest dependency is required.
[[src/builtins/crypto.rs:augmented_project]]

## Description

`crypto::p256Verify` checks whether `signature` is a valid ECDSA signature of
`message` under `publicKey` on the NIST P-256 curve with SHA-256 (FIPS 186). It
returns `TRUE` if and only if the signature verifies for that exact key and
message, and `FALSE` otherwise. The message is hashed with SHA-256 internally, so
pass the raw message bytes, not a digest. [[src/target/shared/code/crypto_ec.rs:ec_call]]

`publicKey` is the 65-byte SEC1 uncompressed point `0x04 || X || Y`, where `X` and
`Y` are the two 32-byte big-endian affine coordinates — exactly the `publicKey`
field returned by `crypto::generateP256`. `signature` is an ASN.1 DER
`Ecdsa-Sig-Value` (X9.62), as produced by `crypto::p256Sign`. Verification depends
only on the three inputs; the private signing key is not required.
[[src/target/shared/code/crypto_ec/openssl.rs:verify]]

A failed verdict is distinguished from a malformed key. A valid-length public key
paired with a signature that simply does not match returns `FALSE` — a normal
outcome, not an error. But a `publicKey` that is not a well-formed 65-byte P-256
SEC1 point (wrong length, or bytes that do not decode to a valid curve point)
raises `ErrInvalidArgument` rather than returning a verdict. A malformed
`signature` that the platform cannot parse also verifies as `FALSE`.
[[src/target/shared/code/crypto_ec/openssl.rs:verify]]

Verification is total and platform-independent: the same
`(publicKey, message, signature)` triple yields the same verdict on macOS and
Linux, on aarch64 and x86-64. The NIST curves bind the platform key API —
`SecKeyVerifySignature` with `kSecKeyAlgorithmECDSASignatureMessageX962SHA256` on
macOS, and OpenSSL `EVP_DigestVerify` on Linux. Keys and DER signatures are
wire-compatible across platforms and with OpenSSL / pyca, so a signature made on
one system verifies on another. ECDSA signing is non-deterministic (a fresh nonce
per call), but a signature and its verdict do not depend on that nonce.
[[src/target/shared/code/crypto_ec.rs:macos_algorithm]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `publicKey` | `List OF Byte` | The P-256 public key as the 65-byte SEC1 uncompressed point `0x04 \|\| X \|\| Y` (the `publicKey` field of a `crypto::generateP256` key pair). Any other length, or bytes that do not decode to a valid curve point, is an error, not a `FALSE` verdict. [[src/target/shared/code/crypto_ec/openssl.rs:verify]] |
| `message` | `List OF Byte` | The raw message bytes whose signature is being verified. Any length is accepted, including empty; hashed with SHA-256 internally, so no pre-hashing is required. |
| `signature` | `List OF Byte` | The candidate signature as an ASN.1 DER (X9.62) `Ecdsa-Sig-Value`. A signature the platform cannot parse verifies as `FALSE`. |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` if `signature` is a valid P-256/SHA-256 signature of `message` under `publicKey`; `FALSE` if it is not (including an unparsable signature). A `FALSE` result is a normal outcome, not an error. [[src/builtins/crypto.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `publicKey` is not a well-formed 65-byte P-256 SEC1 point — either not exactly 65 bytes, or bytes that do not decode to a valid curve point. A valid-length key with a non-matching signature returns `FALSE` instead. [[src/target/shared/code/crypto_ec/openssl.rs:verify]] |

## Examples

Generate a key, sign a message, and verify the signature:

```
IMPORT crypto

LET kp AS crypto::KeyPair = crypto::generateP256()
LET sig AS List OF Byte = crypto::p256Sign(kp.privateKey, message)
LET ok AS Boolean = crypto::p256Verify(kp.publicKey, message, sig)
```

A tampered message fails verification (returns `FALSE`, not an error):

```
IMPORT crypto

LET ok AS Boolean = crypto::p256Verify(kp.publicKey, altered, sig)
```

## See also

- `mfb man crypto p256Sign`
- `mfb man crypto generateP256`
- `mfb man crypto p384Verify`
- `mfb man crypto ed25519Verify`
- `mfb man encoding hexEncode`

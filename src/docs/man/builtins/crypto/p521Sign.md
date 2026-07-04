# p521Sign

ECDSA-sign a message with a NIST P-521 private key (FIPS 186).

## Synopsis

```
crypto::p521Sign(privateKey AS List OF Byte, message AS List OF Byte) AS List OF Byte
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

`crypto::p521Sign` produces an ECDSA signature over `message` using `privateKey`
on the NIST P-521 curve (FIPS 186), hashing the message with SHA-512 internally.
The result is an ASN.1 DER `Ecdsa-Sig-Value` (X9.62) returned as a `List OF Byte`.
Verify it later with `crypto::p521Verify` given the matching public key.
[[src/target/shared/code/crypto_ec/openssl.rs:sign]]

`privateKey` is the 199-byte wire form `0x04 || X || Y || K` — the 133-byte
uncompressed point (`0x04` prefix plus the two 66-byte field elements `X` and `Y`)
followed by the 66-byte secret scalar `K`. This is exactly the `privateKey` field
returned by `crypto::generateP521`. `message` is the raw bytes to sign; it is
hashed with SHA-512 as part of the platform signing call, so the caller does not
pre-hash it. The DER-encoded signature is variable length (roughly 137–139 bytes),
since the encoding of the two integers `r` and `s` depends on their leading bits.
[[src/target/shared/code/crypto_ec/openssl.rs:params]]

The NIST curves bind the platform key API: `SecKeyCreateSignature` with
`kSecKeyAlgorithmECDSASignatureMessageX962SHA512` on macOS, and OpenSSL
`EVP_PKEY` signing on Linux. The DER signature is wire-compatible across
platforms and with OpenSSL / pyca. Unlike Ed25519, ECDSA signing is
**non-deterministic**: a fresh random nonce is drawn per call, so signing the same
`(privateKey, message)` twice yields two different signatures. Both verify
correctly. [[src/target/shared/code/crypto_ec.rs:macos_algorithm]]

**Secret safety.** `privateKey` embeds the secret scalar `K`. Anyone who holds it
can forge signatures. Never log it, and treat `typeName` / `toString` /
diagnostics as non-security boundaries. To store or display a signature, stringify
its bytes with `encoding::hexEncode` (lowercase hex) or `encoding::base64Encode`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `privateKey` | `List OF Byte` | The 199-byte P-521 private key in the `0x04 \|\| X \|\| Y \|\| K` wire form (the `privateKey` field of a `crypto::generateP521` key pair). Must be exactly 199 bytes: the 133-byte point followed by the 66-byte scalar. [[src/target/shared/code/crypto_ec/openssl.rs:sign]] |
| `message` | `List OF Byte` | The raw bytes to sign. Any length; hashed with SHA-512 internally, so no pre-hashing is required. |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The ASN.1 DER (X9.62) `Ecdsa-Sig-Value` ECDSA signature. Variable length (roughly 137–139 bytes) depending on the encoding of the `r` and `s` integers. [[src/builtins/crypto.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `privateKey` is not exactly 199 bytes (the P-521 point length plus field length). [[src/target/shared/code/crypto_ec/openssl.rs:sign]] |
| `77050000` | `ErrUnknown` | The platform signing call itself fails (e.g. the key material does not decode to a valid P-521 private key). [[src/target/shared/code/crypto_ec/openssl.rs:sign]] |
| `77010001` | `ErrOutOfMemory` | An internal working buffer cannot be allocated. [[src/target/shared/code/crypto_ec/openssl.rs:sign]] |

## Examples

Generate a key, sign a message, and verify the signature:

```
IMPORT crypto

LET kp AS crypto::KeyPair = crypto::generateP521()
LET sig AS List OF Byte = crypto::p521Sign(kp.privateKey, message)
LET ok AS Boolean = crypto::p521Verify(kp.publicKey, message, sig)
```

Display a signature as hex:

```
IMPORT crypto
IMPORT encoding

LET sig AS List OF Byte = crypto::p521Sign(kp.privateKey, message)
PRINT encoding::hexEncode(sig)
```

## See also

- `mfb man crypto p521Verify`
- `mfb man crypto generateP521`
- `mfb man crypto p384Sign`
- `mfb man crypto p256Sign`
- `mfb man crypto ed25519Sign`
- `mfb man encoding hexEncode`

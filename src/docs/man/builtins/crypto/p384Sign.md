# p384Sign

ECDSA-sign a message with a NIST P-384 private key (FIPS 186).

## Synopsis

```
crypto::p384Sign(privateKey AS List OF Byte, message AS List OF Byte) AS List OF Byte
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

`crypto::p384Sign` produces an ECDSA signature over `message` using `privateKey`
on the NIST P-384 curve (FIPS 186), hashing the message with SHA-384 internally.
The result is an ASN.1 DER `Ecdsa-Sig-Value` (X9.62) returned as a `List OF Byte`.
Verify it later with `crypto::p384Verify` given the matching public key.
[[src/target/shared/code/crypto_ec.rs:ec_call]]

`privateKey` is the 145-byte wire form `0x04 || X || Y || K` — the 97-byte
uncompressed point (`0x04` prefix plus the two 48-byte field elements `X` and `Y`)
followed by the 48-byte secret scalar `K`. This is exactly the `privateKey` field
returned by `crypto::generateP384`. `message` is the raw bytes to sign; it is
hashed with SHA-384 as part of the platform signing call, so the caller does not
pre-hash it. The DER-encoded signature is variable length (roughly 102–104 bytes),
since the encoding of the two integers `r` and `s` depends on their leading bits.
[[src/target/shared/code/crypto_ec/openssl.rs:params]]

The NIST curves bind the platform key API: `SecKeyCreateSignature` with
`kSecKeyAlgorithmECDSASignatureMessageX962SHA384` on macOS, and OpenSSL
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
| `privateKey` | `List OF Byte` | The 145-byte P-384 private key in the `0x04 \|\| X \|\| Y \|\| K` wire form (the `privateKey` field of a `crypto::generateP384` key pair). Must be exactly 145 bytes: the 97-byte point followed by the 48-byte scalar. [[src/target/shared/code/crypto_ec/openssl.rs:sign]] |
| `message` | `List OF Byte` | The raw bytes to sign. Any length; hashed with SHA-384 internally, so no pre-hashing is required. |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The ASN.1 DER (X9.62) `Ecdsa-Sig-Value` ECDSA signature. Variable length (roughly 102–104 bytes) depending on the encoding of the `r` and `s` integers. [[src/builtins/crypto.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `privateKey` is not exactly 145 bytes (the P-384 point length plus field length), or the 145 bytes do not decode to a valid P-384 private key. [[src/target/shared/code/crypto_ec/openssl.rs:sign]] [[src/target/shared/code/crypto_ec/macos.rs:sign]] |
| `77050000` | `ErrUnknown` | The platform signing call itself fails (the OpenSSL / Security.framework signing operation returns an error, or a required library symbol cannot be loaded). [[src/target/shared/code/crypto_ec/openssl.rs:sign]] [[src/target/shared/code/crypto_ec/macos.rs:sign]] |
| `77010001` | `ErrOutOfMemory` | An internal working buffer cannot be allocated. [[src/target/shared/code/crypto_ec/openssl.rs:sign]] [[src/target/shared/code/crypto_ec/macos.rs:sign]] |

## Examples

Generate a key, sign a message, and verify the signature:

```
IMPORT crypto

LET kp AS crypto::KeyPair = crypto::generateP384()
LET sig AS List OF Byte = crypto::p384Sign(kp.privateKey, message)
LET ok AS Boolean = crypto::p384Verify(kp.publicKey, message, sig)
```

Display a signature as hex:

```
IMPORT crypto
IMPORT encoding

LET sig AS List OF Byte = crypto::p384Sign(kp.privateKey, message)
PRINT encoding::hexEncode(sig)
```

## See also

- `mfb man crypto p384Verify`
- `mfb man crypto generateP384`
- `mfb man crypto p256Sign`
- `mfb man crypto p521Sign`
- `mfb man crypto ed25519Sign`
- `mfb man encoding hexEncode`

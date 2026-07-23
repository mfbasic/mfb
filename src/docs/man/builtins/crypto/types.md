# types

the crypto package record types

## Synopsis

```
crypto::Sealed
crypto::KeyPair
```

## Package

crypto

## Imports

```
IMPORT crypto
```

`crypto` is a built-in package, so `IMPORT crypto` needs no manifest
dependency. [[src/builtins/crypto.rs:augmented_project]]

## Description

The `crypto` package defines two record types. Both are plain, copyable records
whose fields are `List OF Byte` and are read with ordinary field access; they
raise no errors of their own. [[src/builtins/crypto_hash.mfb:Sealed]]

`crypto::Sealed` is the output of authenticated encryption (AEAD): it pairs a
ciphertext with its authentication tag. It is produced by `crypto::aes256GcmSeal`
and `crypto::chacha20Poly1305Seal`, and its two fields are passed back to the
matching open operation (`crypto::aes256GcmOpen`, `crypto::chacha20Poly1305Open`)
to verify and decrypt.

`crypto::KeyPair` is the output of public-key key generation: it pairs a private
key with its matching public key. It is produced by `crypto::generateEd25519` and
by the NIST-curve generators `crypto::generateP256`, `crypto::generateP384`, and
`crypto::generateP521`. Its `privateKey` feeds the signing operations and its
`publicKey` feeds verification. The byte encodings are identical on every target
and are wire-compatible with OpenSSL and pyca.

For the NIST curves the fields use self-contained SEC1 uncompressed encodings:
`privateKey` is `0x04 || X || Y || K` (the uncompressed point followed by the
big-endian scalar `K` — 97 bytes for P-256, 145 for P-384, 199 for P-521) and
`publicKey` is `0x04 || X || Y` (65 / 97 / 133 bytes for P-256 / P-384 / P-521).
For Ed25519, `privateKey` is the 32-byte seed and `publicKey` is the 32-byte
public key.

**Secret safety.** `crypto::KeyPair.privateKey` holds secret key material. Never
log, print, or serialize it in diagnostics — `typeName`, `toString`, and error
messages are not security boundaries. Keep the private key confidential and
distribute only `publicKey`.

## Types

### crypto::Sealed

The result of AEAD sealing: an authenticated ciphertext. [[src/builtins/crypto_hash.mfb:Sealed]]

| Field | Type | Description |
| --- | --- | --- |
| `ciphertext` | `List OF Byte` | The encrypted bytes; the same length as the original plaintext. |
| `tag` | `List OF Byte` | The 16-byte authentication tag (AES-256-GCM or ChaCha20-Poly1305). Binds the ciphertext and any additional authenticated data; checked in constant time by the open operation, which fails closed on mismatch. |

### crypto::KeyPair

A generated public/private key pair. [[src/builtins/crypto_hash.mfb:KeyPair]]

| Field | Type | Description |
| --- | --- | --- |
| `privateKey` | `List OF Byte` | The private key — secret. For NIST curves, the SEC1 point followed by the big-endian scalar (97 / 145 / 199 bytes for P-256 / P-384 / P-521); for Ed25519, the 32-byte seed. Never log or serialize it. See the encodings in the Description. |
| `publicKey` | `List OF Byte` | The public key — safe to publish and distribute. For NIST curves, the uncompressed SEC1 point (65 / 97 / 133 bytes for P-256 / P-384 / P-521); for Ed25519, the 32-byte public key. |

## See also

- `mfb man crypto`
- `mfb man crypto aes256GcmSeal`
- `mfb man crypto chacha20Poly1305Seal`
- `mfb man crypto generateEd25519`
- `mfb man crypto ed25519Sign`
- `mfb man encoding` — hex/Base64 stringification of digests and keys

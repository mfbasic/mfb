# generateEd25519

Generate a random Ed25519 signing key pair (RFC 8032).

## Synopsis

```
crypto::generateEd25519() AS crypto::KeyPair
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

`crypto::generateEd25519` creates a fresh Ed25519 signing key pair for use with
`crypto::ed25519Sign` and `crypto::ed25519Verify`, following RFC 8032 (PureEdDSA
over Curve25519). It takes no arguments and returns a `crypto::KeyPair` record
with two fields: [[src/builtins/crypto_package.mfb:__crypto_generateEd25519]]

- `privateKey` — the 32-byte Ed25519 secret seed (`List OF Byte`).
- `publicKey` — the 32-byte Ed25519 public key (`List OF Byte`), derived from the
  seed by SHA-512, scalar clamping, and scalar-base multiplication.
  [[src/builtins/crypto_package.mfb:__crypto_ed25519Public]]

The secret seed is drawn from the OS CSPRNG via `crypto::randomBytes(32)`, so the
result is random and non-reproducible: every call yields a different key pair.
There is no seeded or deterministic form; to persist a key, store the returned
bytes yourself. [[src/builtins/crypto_package.mfb:__crypto_generateEd25519]]

Ed25519 is a portable software core, so keys and the algorithm behave identically
on every target (macOS/Linux, aarch64/x86-64) and use no platform crypto library.
[[src/builtins/crypto.rs:implementation_name]]

**Secret safety.** The `privateKey` field is sensitive secret material. Anyone who
holds it can forge signatures. Never log a `KeyPair`, and treat `typeName` /
`toString` / diagnostics as non-security boundaries. The `publicKey` is safe to
share; distribute it to verifiers.

To display or store a key, stringify its bytes with `encoding::hexEncode`
(lowercase hex) or `encoding::base64Encode`.

## Parameters

None.

## Return value

| Type | Description |
| --- | --- |
| `crypto::KeyPair` | A record whose `privateKey` is the 32-byte Ed25519 secret seed and whose `publicKey` is the 32-byte Ed25519 public key. [[src/builtins/crypto_package.mfb:KeyPair]] |

## Errors

No errors.

## Examples

Generate a key pair and print the public key as hex:

```
IMPORT crypto
IMPORT encoding

LET kp AS crypto::KeyPair = crypto::generateEd25519()
PRINT encoding::hexEncode(kp.publicKey)
```

Sign a message with the freshly generated key:

```
IMPORT crypto

LET kp AS crypto::KeyPair = crypto::generateEd25519()
LET sig AS List OF Byte = crypto::ed25519Sign(kp.privateKey, message)
```

## See also

- `mfb man crypto ed25519Sign`
- `mfb man crypto ed25519Verify`
- `mfb man crypto generateP256`
- `mfb man encoding hexEncode`

# ed25519Sign

Sign a message with an Ed25519 private key (RFC 8032).

## Synopsis

```
crypto::ed25519Sign(privateKey AS List OF Byte, message AS List OF Byte) AS List OF Byte
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

`crypto::ed25519Sign` produces an Ed25519 signature over `message` using
`privateKey`, following RFC 8032 (PureEdDSA over Curve25519). The result is a
fixed 64-byte signature returned as a `List OF Byte`, the concatenation of the
32-byte `R` point and the 32-byte `S` scalar. Verify it later with
`crypto::ed25519Verify` given the matching public key.
[[src/builtins/crypto_package.mfb:__crypto_ed25519Sign]]

`privateKey` is the 32-byte Ed25519 secret seed — exactly the `privateKey` field
returned by `crypto::generateEd25519`. The public key, nonce prefix, and signing
scalar are all derived from this seed by SHA-512, so no separate public key is
passed in. `message` is the raw bytes to sign; Ed25519 is a PureEdDSA scheme, so
the whole message is signed directly with no pre-hashing required from the
caller. [[src/builtins/crypto_package.mfb:__crypto_ed25519Sign]]

Ed25519 signing is deterministic: the per-signature nonce is derived from the
key and the message rather than from randomness, so signing the same
`(privateKey, message)` always yields the same 64-byte signature. This holds on
every target (macOS/Linux, aarch64/x86-64), since Ed25519 is a portable software
core with byte-identical output and uses no platform crypto library.
[[src/builtins/crypto.rs:implementation_name]]

**Secret safety.** `privateKey` is sensitive secret material. Anyone who holds
it can forge signatures. Never log it, and treat `typeName` / `toString` /
diagnostics as non-security boundaries. To store or display a signature,
stringify its bytes with `encoding::hexEncode` (lowercase hex) or
`encoding::base64Encode`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `privateKey` | `List OF Byte` | The 32-byte Ed25519 secret seed (the `privateKey` field of a `crypto::generateEd25519` key pair). Must be exactly 32 bytes. [[src/builtins/crypto_package.mfb:__crypto_ed25519Sign]] |
| `message` | `List OF Byte` | The raw bytes to sign. Any length, signed directly without pre-hashing. |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The 64-byte Ed25519 signature: the 32-byte `R` point followed by the 32-byte `S` scalar. [[src/builtins/crypto_package.mfb:__crypto_ed25519Sign]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `privateKey` is not exactly 32 bytes long. [[src/builtins/crypto_package.mfb:__crypto_ed25519Sign]] |

## Examples

Generate a key, sign a message, and verify the signature:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET kp AS crypto::KeyPair = crypto::generateEd25519()
  LET sig AS List OF Byte = crypto::ed25519Sign(kp.privateKey, message)
  LET ok AS Boolean = crypto::ed25519Verify(kp.publicKey, message, sig)
END SUB
```

Display a signature as hex:

```
IMPORT crypto
IMPORT strings
IMPORT encoding
IMPORT io

SUB main()
  LET kp AS crypto::KeyPair = crypto::generateEd25519()
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::ed25519Sign(kp.privateKey, message)
  io::print(encoding::hexEncode(sig))
END SUB
```

## See also

- `mfb man crypto ed25519Verify`
- `mfb man crypto generateEd25519`
- `mfb man crypto p256Sign`
- `mfb man encoding hexEncode`

# ed25519Verify

Verify an Ed25519 signature over a message with a public key (RFC 8032).

## Synopsis

```
crypto::ed25519Verify(publicKey AS List OF Byte, message AS List OF Byte, signature AS List OF Byte) AS Boolean
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

`crypto::ed25519Verify` checks whether `signature` is a valid Ed25519 signature
of `message` under `publicKey`, following RFC 8032 (PureEdDSA over Curve25519).
It returns `TRUE` if and only if the signature verifies for that exact key and
message, and `FALSE` otherwise. Verification depends only on the three inputs;
the matching signing key is not required.
[[src/builtins/crypto_package.mfb:__crypto_ed25519Verify]]

`publicKey` is the 32-byte Ed25519 public key — exactly the `publicKey` field
returned by `crypto::generateEd25519`. `message` is the raw message bytes that
were signed; Ed25519 is a PureEdDSA scheme, so the whole message is hashed
internally and no pre-hashing is applied by the caller. `signature` is the
64-byte signature produced by `crypto::ed25519Sign` or any interoperating
implementation — the concatenation of the 32-byte `R` point and the 32-byte `S`
scalar. [[src/builtins/crypto_package.mfb:__crypto_ed25519Verify]]

Verification is total and never raises: it always returns a `TRUE`/`FALSE`
verdict. A `publicKey` that is not exactly 32 bytes, a `signature` that is not
exactly 64 bytes, a public key that does not decode to a valid curve point, a
signature whose `S` scalar is not canonical (`S >= L`, the group order — such a
signature is malleable and is rejected so the signature bytes remain a stable
identity, bug-269 / CRY-02), or a signature that simply does not match all return
`FALSE` — a failed verdict is a normal outcome, not an error.
[[src/builtins/crypto_package.mfb:__crypto_ed25519Verify]]

Verification is deterministic and platform-independent: the same
`(publicKey, message, signature)` triple yields the same verdict on every target
(macOS/Linux, aarch64/x86-64), because Ed25519 is a portable software core with
byte-identical behavior and uses no platform crypto library. Signatures
interoperate across platforms and with standard toolkits.
[[src/builtins/crypto.rs:implementation_name]]

The final comparison of the recomputed `R` point against the signature's `R` is
done with a constant-time byte compare, so a matching-length verification does
not leak timing information about how far the two points agree.
[[src/builtins/crypto_package.mfb:__crypto_constantTimeEqual]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `publicKey` | `List OF Byte` | The Ed25519 public key (the `publicKey` field of a `crypto::generateEd25519` key pair). Any length other than exactly 32 bytes yields a `FALSE` verdict. [[src/builtins/crypto_package.mfb:__crypto_ed25519Verify]] |
| `message` | `List OF Byte` | The raw message bytes whose signature is being verified. Any length is accepted, including empty; the whole message is hashed internally without pre-hashing. |
| `signature` | `List OF Byte` | The candidate signature. Any length other than exactly 64 bytes yields a `FALSE` verdict. [[src/builtins/crypto_package.mfb:__crypto_ed25519Verify]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` if `signature` is a valid Ed25519 signature of `message` under `publicKey`; `FALSE` otherwise (including a mis-sized key or signature). A `FALSE` result is a normal outcome, not an error. [[src/builtins/crypto_package.mfb:__crypto_ed25519Verify]] |

## Errors

No errors.

## Examples

Generate a key, sign a message, and verify the signature:

```
IMPORT crypto
IMPORT strings

LET kp AS crypto::KeyPair = crypto::generateEd25519()
LET message AS List OF Byte = strings::toBytes("attack at dawn")
LET sig AS List OF Byte = crypto::ed25519Sign(kp.privateKey, message)
LET ok AS Boolean = crypto::ed25519Verify(kp.publicKey, message, sig)
PRINT ok
```

A tampered message fails verification (returns `FALSE`, not an error):

```
IMPORT crypto
IMPORT strings

LET altered AS List OF Byte = strings::toBytes("attack at dusk")
LET bad AS Boolean = crypto::ed25519Verify(kp.publicKey, altered, sig)
PRINT bad
```

## See also

- `mfb man crypto ed25519Sign`
- `mfb man crypto generateEd25519`
- `mfb man crypto p256Verify`
- `mfb man crypto constantTimeEqual`

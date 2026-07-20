# hkdfSha256

Derive key material with HKDF (RFC 5869) instantiated over HMAC-SHA-256.

## Synopsis

```
crypto::hkdfSha256(ikm AS List OF Byte, salt AS List OF Byte, info AS List OF Byte, length AS Integer) AS List OF Byte
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

`crypto::hkdfSha256` is the HKDF key-derivation function of RFC 5869 instantiated
over HMAC-SHA-256. It turns input keying material of arbitrary quality into one
or more cryptographically strong keys of a chosen length. HKDF runs in two
phases: an **extract** step folds `ikm` and `salt` into a fixed-length
pseudorandom key (the 32-byte HMAC-SHA-256 output), and an **expand** step
stretches that key into `length` output bytes bound to the `info` context.
[[src/builtins/crypto_package.mfb:__crypto_hkdfSha256]]

`salt` is optional in the RFC sense: passing an empty list selects HKDF's default
all-zero salt of one hash block (32 bytes).
[[src/builtins/crypto_package.mfb:__crypto_hkdfSha256]] `info` may also be empty;
when non-empty it domain-separates derived keys, so the same `ikm` can safely
produce independent keys for different purposes.

`length` must be at least 1 and at most `255 * 32 = 8160` bytes — the ceiling
imposed by HKDF-Expand's single-byte block counter over a 32-byte hash. A
`length` of 0 or below, or above 8160, raises `ErrInvalidArgument`.
[[src/builtins/crypto_package.mfb:__crypto_hkdfSha256]]

The function is deterministic and total within its `length` bound: the same four
arguments always yield the same bytes. Because it is a portable software core
computed over the `bits` package, its output is **byte-identical on every
target** (macOS/Linux, aarch64/x86-64) and uses no platform crypto library.
[[src/builtins/crypto.rs:implementation_name]]

HKDF is designed for high-entropy `ikm` (for example a Diffie-Hellman shared
secret). To derive keys from a low-entropy password, use `crypto::pbkdf2Sha256`
instead. Derived key material is raw binary; to display or store it, stringify it
with `encoding::hexEncode` or `encoding::base64Encode`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `ikm` | `List OF Byte` | Input keying material to derive from. Any length is accepted, including the empty list. |
| `salt` | `List OF Byte` | A non-secret salt. Pass an empty list to select HKDF's default all-zero 32-byte salt. |
| `info` | `List OF Byte` | Optional context / application-specific information for domain separation. May be empty. |
| `length` | `Integer` | Number of output bytes to produce. Must be in `1 .. 8160` (`255 * 32`). [[src/builtins/crypto_package.mfb:__crypto_hkdfSha256]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | Exactly `length` pseudorandom bytes of derived key material. [[src/builtins/crypto.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `length` is less than 1 or greater than `8160` (`255 * 32`), the maximum output HKDF-Expand can produce for a 32-byte hash. [[src/builtins/crypto_package.mfb:__crypto_hkdfSha256]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Derive a 32-byte key from a shared secret:

```
IMPORT crypto
IMPORT strings

SUB main()
  LET secret AS List OF Byte = crypto::randomBytes(32)
  LET salt AS List OF Byte = crypto::randomBytes(16)
  LET info AS List OF Byte = strings::toBytes("app v1")
  LET key AS List OF Byte = crypto::hkdfSha256(secret, salt, info, 32)
END SUB
```

An empty salt selects the RFC default all-zero salt:

```
IMPORT crypto

SUB main()
  LET secret AS List OF Byte = crypto::randomBytes(32)
  LET empty AS List OF Byte = []
  LET key AS List OF Byte = crypto::hkdfSha256(secret, empty, empty, 64)
END SUB
```

## See also

- `mfb man crypto hkdfSha512`
- `mfb man crypto pbkdf2Sha256`
- `mfb man crypto hmacSha256`
- `mfb man encoding hexEncode`

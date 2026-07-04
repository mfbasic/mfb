# pbkdf2Sha512

Derive a key from a password with PBKDF2 (RFC 8018) over HMAC-SHA-512.

## Synopsis

```
crypto::pbkdf2Sha512(password AS List OF Byte, salt AS List OF Byte, iterations AS Integer, length AS Integer) AS List OF Byte
crypto::pbkdf2Sha512(password AS String, salt AS List OF Byte, iterations AS Integer, length AS Integer) AS List OF Byte
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

`crypto::pbkdf2Sha512` is the PBKDF2 password-based key-derivation function of
RFC 8018 instantiated over HMAC-SHA-512. It stretches a low-entropy `password`
into `length` bytes of derived key material by iterating the HMAC core
`iterations` times per output block, deliberately making brute-force guessing of
the password proportionally more expensive.
[[src/builtins/crypto_package.mfb:__crypto_pbkdf2Sha512_bytes]]

The output is produced one 64-byte HMAC-SHA-512 block at a time until at least
`length` bytes have accumulated, then truncated to exactly `length` bytes. Each
block folds `salt` and the block index through `iterations` rounds of HMAC,
XOR-accumulating every round into the block.
[[src/builtins/crypto_package.mfb:__crypto_pbkdf2Block512]]

`salt` should be unique per password and need not be secret; a random salt from
`crypto::randomBytes` (16 bytes or more) is recommended, stored alongside the
derived key. `iterations` sets the work factor and directly trades security for
latency; choose the largest value your latency budget tolerates.

This cost is what distinguishes PBKDF2 from HKDF: use PBKDF2 for passwords, and
`crypto::hkdfSha512` for already-high-entropy keying material.

`password` is overloaded: a `String` argument is UTF-8-encoded internally, so
the `String` and `List OF Byte` forms agree for ASCII and UTF-8 text.
[[src/builtins/crypto.rs:implementation_name]] The function is deterministic and
total within its argument bounds — the same inputs always yield the same bytes.
Because it is a portable software core computed over the `bits` package, its
output is **byte-identical on every target** (macOS/Linux, aarch64/x86-64) and
uses no platform crypto library. [[src/builtins/crypto.rs:implementation_name]]

Derived key material is raw binary; to display or store it, stringify it with
`encoding::hexEncode` or `encoding::base64Encode`.

## Overloads

**`crypto::pbkdf2Sha512(password AS List OF Byte, salt AS List OF Byte, iterations AS Integer, length AS Integer) AS List OF Byte`**

Derives from the raw bytes of `password` exactly as given.

**`crypto::pbkdf2Sha512(password AS String, salt AS List OF Byte, iterations AS Integer, length AS Integer) AS List OF Byte`**

Derives from the UTF-8 encoding of the string. It is equivalent to converting
the string to its UTF-8 bytes and deriving from those; the concrete `password`
type selects the `_text` implementation body.
[[src/builtins/crypto.rs:implementation_name]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `password` | `List OF Byte` | The password to derive from, as raw bytes. Any length is accepted. |
| `password` | `String` | A password string whose UTF-8 bytes are used. |
| `salt` | `List OF Byte` | A per-password salt. Should be unique; need not be secret. Any length is accepted. |
| `iterations` | `Integer` | The PBKDF2 iteration count (work factor). Must be at least 1. Larger values are more resistant to brute force and slower. [[src/builtins/crypto_package.mfb:__crypto_pbkdf2Sha512_bytes]] |
| `length` | `Integer` | Number of derived key bytes to produce. Must be at least 1. [[src/builtins/crypto_package.mfb:__crypto_pbkdf2Sha512_bytes]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | Exactly `length` bytes of derived key material. [[src/builtins/crypto.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `iterations` is less than 1, or `length` is less than 1. [[src/builtins/crypto_package.mfb:__crypto_pbkdf2Sha512_bytes]] |

## Type checking

The first argument (`password`) must be either a `List OF Byte` or a `String`;
no other type resolves. `salt` must be a `List OF Byte`, and both `iterations`
and `length` must be `Integer`. Exactly four arguments are required. The return
type is always `List OF Byte`.
[[src/builtins/crypto.rs:resolve_call]] [[src/builtins/crypto.rs:arity]]

## Examples

Derive a 64-byte key from a password string:

```
IMPORT crypto

LET salt AS List OF Byte = crypto::randomBytes(16)
LET key AS List OF Byte = crypto::pbkdf2Sha512("correct horse", salt, 100000, 64)
```

The byte-list form is equivalent for UTF-8 input:

```
IMPORT crypto

LET key AS List OF Byte = crypto::pbkdf2Sha512(passwordBytes, salt, 100000, 64)
```

## See also

- `mfb man crypto pbkdf2Sha256`
- `mfb man crypto hkdfSha512`
- `mfb man crypto hmacSha512`
- `mfb man crypto randomBytes`
- `mfb man encoding hexEncode`

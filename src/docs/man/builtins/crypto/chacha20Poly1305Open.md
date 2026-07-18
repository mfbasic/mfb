# chacha20Poly1305Open

Verify and decrypt a ChaCha20-Poly1305 sealed message, failing closed on any tag mismatch (RFC 8439).

## Synopsis

```
crypto::chacha20Poly1305Open(key AS List OF Byte, nonce AS List OF Byte, ciphertext AS List OF Byte, tag AS List OF Byte) AS List OF Byte
crypto::chacha20Poly1305Open(key AS List OF Byte, nonce AS List OF Byte, ciphertext AS List OF Byte, tag AS List OF Byte, aad AS List OF Byte) AS List OF Byte
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

`crypto::chacha20Poly1305Open` verifies and decrypts a ciphertext produced by
`crypto::chacha20Poly1305Seal`, using the ChaCha20-Poly1305 AEAD construction as
specified by RFC 8439. It recomputes the Poly1305 authentication tag over the
`ciphertext` and the additional authenticated data, compares that tag against the
supplied `tag` in constant time, and returns the recovered plaintext only if they
match. [[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Open]]

The function fails closed. On any tag mismatch it raises
`ErrAuthenticationFailed` and returns no plaintext at all — not a partial or
unverified decryption. The tag is compared with `crypto::constantTimeEqual`, so
the check is content-independent in time and does not leak how much of the tag
matched. A mismatch means the `ciphertext`, `tag`, `nonce`, or `aad` was
altered, truncated, or does not belong to this key; the message must be
rejected. [[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Open]]

`key` must be exactly 32 bytes (a 256-bit key) and `nonce` must be exactly 12
bytes (the 96-bit RFC 8439 nonce); any other length raises `ErrInvalidArgument`.
[[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Open]]
To open successfully, `key`, `nonce`, `ciphertext`, `tag`, and `aad` must all be
identical to those from the sealing call: the `aad` is authenticated but not
carried in the ciphertext, so the same `aad` must be supplied here. `aad`
defaults to the empty list when omitted. [[src/builtins/crypto.rs:default_argument_padding]]
An empty `ciphertext` is valid and recovers an empty plaintext when the tag over
the `aad` alone verifies.

The cipher is a portable software core computed over the `bits` package, so its
behavior is **byte-identical on every target** (macOS/Linux, aarch64/x86-64) and
uses no platform crypto library. [[src/builtins/crypto.rs:implementation_name]]
ChaCha20-Poly1305 is a strong choice on targets without AES hardware
acceleration; AES-256-GCM (`crypto::aes256GcmOpen`) is the interchangeable
alternative.

## Overloads

**`crypto::chacha20Poly1305Open(key AS List OF Byte, nonce AS List OF Byte, ciphertext AS List OF Byte, tag AS List OF Byte) AS List OF Byte`**

Opens a message sealed with no additional authenticated data; `aad` defaults to
the empty list. [[src/builtins/crypto.rs:default_argument_padding]]

**`crypto::chacha20Poly1305Open(key AS List OF Byte, nonce AS List OF Byte, ciphertext AS List OF Byte, tag AS List OF Byte, aad AS List OF Byte) AS List OF Byte`**

Opens a message that additionally authenticated `aad` at seal time. The `aad`
must be byte-for-byte identical to the value passed to
`crypto::chacha20Poly1305Seal`, or verification fails.
[[src/builtins/crypto.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `key` | `List OF Byte` | The 256-bit ChaCha20 key. Must be exactly 32 bytes and identical to the sealing key. This value is secret. [[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Open]] |
| `nonce` | `List OF Byte` | The 96-bit RFC 8439 nonce. Must be exactly 12 bytes and identical to the nonce used to seal the message. [[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Open]] |
| `ciphertext` | `List OF Byte` | The encrypted bytes, from `crypto::Sealed.ciphertext`. May be empty. |
| `tag` | `List OF Byte` | The 16-byte Poly1305 authentication tag, from `crypto::Sealed.tag`. |
| `aad` | `List OF Byte` | Optional additional authenticated data. Must be identical to the `aad` passed when sealing. Defaults to the empty list. [[src/builtins/crypto.rs:default_argument_padding]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The recovered plaintext, the same length as the original message, returned only when the tag verifies. An empty message recovers an empty list. [[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Open]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `key` is not exactly 32 bytes, or `nonce` is not exactly 12 bytes. Both length checks run before verification. [[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Open]] |
| `77050016` | `ErrAuthenticationFailed` | The authentication tag does not verify — the `ciphertext`, `tag`, `nonce`, or `aad` was altered or does not belong to this key. No plaintext is returned. [[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Open]] |

## Examples

Round-trip: seal then open:

```
IMPORT crypto

LET key AS List OF Byte = crypto::randomBytes(32)
LET nonce AS List OF Byte = crypto::randomBytes(12)
LET box AS crypto::Sealed = crypto::chacha20Poly1305Seal(key, nonce, plaintext)
LET clear AS List OF Byte = crypto::chacha20Poly1305Open(key, nonce, box.ciphertext, box.tag)
```

Open a message sealed with additional authenticated data (a header); the same
header must be supplied:

```
IMPORT crypto

LET box AS crypto::Sealed = crypto::chacha20Poly1305Seal(key, nonce, plaintext, header)
LET clear AS List OF Byte = crypto::chacha20Poly1305Open(key, nonce, box.ciphertext, box.tag, header)
```

A tampered ciphertext, tag, or aad raises `ErrAuthenticationFailed` and returns
nothing:

```
IMPORT crypto

' If box.tag has been altered in transit, this call fails closed:
LET clear AS List OF Byte = crypto::chacha20Poly1305Open(key, nonce, box.ciphertext, box.tag)
```

## See also

- `mfb man crypto chacha20Poly1305Seal`
- `mfb man crypto aes256GcmOpen`
- `mfb man crypto constantTimeEqual`
- `mfb man crypto randomBytes`

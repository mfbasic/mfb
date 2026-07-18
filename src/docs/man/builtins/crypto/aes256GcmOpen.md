# aes256GcmOpen

Verify and decrypt an AES-256-GCM sealed message, failing closed on any tag mismatch (NIST SP 800-38D).

## Synopsis

```
crypto::aes256GcmOpen(key AS List OF Byte, nonce AS List OF Byte, ciphertext AS List OF Byte, tag AS List OF Byte) AS List OF Byte
crypto::aes256GcmOpen(key AS List OF Byte, nonce AS List OF Byte, ciphertext AS List OF Byte, tag AS List OF Byte, aad AS List OF Byte) AS List OF Byte
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

`crypto::aes256GcmOpen` verifies and decrypts a ciphertext produced by
`crypto::aes256GcmSeal`, using AES-256 in Galois/Counter Mode (GCM) as specified
by NIST SP 800-38D. It recomputes the authentication tag over the `ciphertext`
and the additional authenticated data, compares that tag against the supplied
`tag` in constant time, and returns the recovered plaintext only if they match.
[[src/builtins/crypto_package.mfb:__crypto_aes256GcmOpen]]

The function fails closed. On any tag mismatch it raises
`ErrAuthenticationFailed` and returns no plaintext at all — not a partial or
unverified decryption. The tag is compared with `crypto::constantTimeEqual`, so
the check is content-independent in time and does not leak how much of the tag
matched. A mismatch means the `ciphertext`, `tag`, `nonce`, or `aad` was
altered, truncated, or does not belong to this key; the message must be
rejected. [[src/builtins/crypto_package.mfb:__crypto_aes256GcmOpen]]

`key` must be exactly 32 bytes (a 256-bit key) and `nonce` must be exactly 12
bytes (the standard 96-bit GCM nonce); any other length raises
`ErrInvalidArgument`. [[src/builtins/crypto_package.mfb:__crypto_aes256GcmOpen]]
To open successfully, `key`, `nonce`, `ciphertext`, `tag`, and `aad` must all be
identical to those from the sealing call: the `aad` is authenticated but not
carried in the ciphertext, so the same `aad` must be supplied here. `aad`
defaults to the empty list when omitted. [[src/builtins/crypto.rs:default_argument_padding]]
An empty `ciphertext` is valid and recovers an empty plaintext when the tag over
the `aad` alone verifies.

The cipher is a portable software core computed over the `bits` package, so its
behavior is **byte-identical on every target** (macOS/Linux, aarch64/x86-64) and
uses no platform crypto library. [[src/builtins/crypto.rs:implementation_name]]

## Overloads

**`crypto::aes256GcmOpen(key AS List OF Byte, nonce AS List OF Byte, ciphertext AS List OF Byte, tag AS List OF Byte) AS List OF Byte`**

Opens a message sealed with no additional authenticated data; `aad` defaults to
the empty list. [[src/builtins/crypto.rs:default_argument_padding]]

**`crypto::aes256GcmOpen(key AS List OF Byte, nonce AS List OF Byte, ciphertext AS List OF Byte, tag AS List OF Byte, aad AS List OF Byte) AS List OF Byte`**

Opens a message that additionally authenticated `aad` at seal time. The `aad`
must be byte-for-byte identical to the value passed to `crypto::aes256GcmSeal`,
or verification fails. [[src/builtins/crypto.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `key` | `List OF Byte` | The 256-bit AES key. Must be exactly 32 bytes and identical to the sealing key. This value is secret. [[src/builtins/crypto_package.mfb:__crypto_aes256GcmOpen]] |
| `nonce` | `List OF Byte` | The 96-bit GCM nonce. Must be exactly 12 bytes and identical to the nonce used to seal the message. [[src/builtins/crypto_package.mfb:__crypto_aes256GcmOpen]] |
| `ciphertext` | `List OF Byte` | The encrypted bytes, from `crypto::Sealed.ciphertext`. May be empty. |
| `tag` | `List OF Byte` | The 16-byte authentication tag, from `crypto::Sealed.tag`. |
| `aad` | `List OF Byte` | Optional additional authenticated data. Must be identical to the `aad` passed when sealing. Defaults to the empty list. [[src/builtins/crypto.rs:default_argument_padding]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The recovered plaintext, the same length as the original message, returned only when the tag verifies. An empty message recovers an empty list. [[src/builtins/crypto_package.mfb:__crypto_aes256GcmOpen]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050016` | `ErrAuthenticationFailed` | The authentication tag does not verify — the `ciphertext`, `tag`, `nonce`, or `aad` was altered or does not belong to this key. No plaintext is returned. [[src/builtins/crypto_package.mfb:__crypto_aes256GcmOpen]] [[src/target/shared/code/error_constants.rs:176]] |
| `77050002` | `ErrInvalidArgument` | `key` is not exactly 32 bytes, or `nonce` is not exactly 12 bytes. [[src/builtins/crypto_package.mfb:__crypto_aes256GcmOpen]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Round-trip: seal then open:

```
IMPORT crypto

LET key AS List OF Byte = crypto::randomBytes(32)
LET nonce AS List OF Byte = crypto::randomBytes(12)
LET box AS crypto::Sealed = crypto::aes256GcmSeal(key, nonce, plaintext)
LET clear AS List OF Byte = crypto::aes256GcmOpen(key, nonce, box.ciphertext, box.tag)
```

Open a message sealed with additional authenticated data (a header); the same
header must be supplied:

```
IMPORT crypto

LET box AS crypto::Sealed = crypto::aes256GcmSeal(key, nonce, plaintext, header)
LET clear AS List OF Byte = crypto::aes256GcmOpen(key, nonce, box.ciphertext, box.tag, header)
```

A tampered ciphertext, tag, or aad raises `ErrAuthenticationFailed` and returns
nothing:

```
IMPORT crypto

' If box.tag has been altered in transit, this call fails closed:
LET clear AS List OF Byte = crypto::aes256GcmOpen(key, nonce, box.ciphertext, box.tag)
```

## See also

- `mfb man crypto aes256GcmSeal`
- `mfb man crypto chacha20Poly1305Open`
- `mfb man crypto constantTimeEqual`
- `mfb man crypto randomBytes`

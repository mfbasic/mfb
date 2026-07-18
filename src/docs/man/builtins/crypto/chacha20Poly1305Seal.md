# chacha20Poly1305Seal

Encrypt and authenticate a message with the ChaCha20-Poly1305 AEAD construction (RFC 8439).

## Synopsis

```
crypto::chacha20Poly1305Seal(key AS List OF Byte, nonce AS List OF Byte, plaintext AS List OF Byte) AS crypto::Sealed
crypto::chacha20Poly1305Seal(key AS List OF Byte, nonce AS List OF Byte, plaintext AS List OF Byte, aad AS List OF Byte) AS crypto::Sealed
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

`crypto::chacha20Poly1305Seal` encrypts and authenticates `plaintext` with the
ChaCha20-Poly1305 AEAD construction, as specified by RFC 8439. It returns a
`crypto::Sealed` record holding the ciphertext (the same length as `plaintext`)
and a 16-byte Poly1305 authentication tag that binds the ciphertext together with
any additional authenticated data. The tag is later checked by
`crypto::chacha20Poly1305Open`, which fails closed on any mismatch.
[[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Seal]]

`key` must be exactly 32 bytes (a 256-bit key) and `nonce` must be exactly 12
bytes (the 96-bit RFC 8439 nonce); any other length raises `ErrInvalidArgument`.
[[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Seal]]
The optional `aad` (additional authenticated data) is authenticated but not
encrypted: it is covered by the tag yet absent from the ciphertext, so a receiver
must supply the identical `aad` to `crypto::chacha20Poly1305Open`. `aad` defaults
to the empty list when omitted. [[src/builtins/crypto.rs:default_argument_padding]]
`plaintext` may be empty, in which case the result carries an empty ciphertext and
a tag over the `aad` alone.

The cipher is a portable software core computed over the `bits` package, so its
output is **byte-identical on every target** (macOS/Linux, aarch64/x86-64) and
uses no platform crypto library. [[src/builtins/crypto.rs:implementation_name]]
ChaCha20-Poly1305 is a strong choice on targets without AES hardware
acceleration; AES-256-GCM (`crypto::aes256GcmSeal`) is the interchangeable
alternative.

Nonce uniqueness is mandatory. ChaCha20-Poly1305 is catastrophically insecure if
a `(key, nonce)` pair is ever reused: repeating a nonce under the same key leaks
the XOR of the plaintexts and can expose the Poly1305 authentication key, breaking
both confidentiality and integrity. Never reuse a `(key, nonce)` pair — generate a
fresh nonce for every message with `crypto::randomBytes(12)` and store or transmit
it alongside the ciphertext (the nonce is not secret).

## Overloads

**`crypto::chacha20Poly1305Seal(key AS List OF Byte, nonce AS List OF Byte, plaintext AS List OF Byte) AS crypto::Sealed`**

Seals `plaintext` with no additional authenticated data; `aad` defaults to the
empty list. [[src/builtins/crypto.rs:default_argument_padding]]

**`crypto::chacha20Poly1305Seal(key AS List OF Byte, nonce AS List OF Byte, plaintext AS List OF Byte, aad AS List OF Byte) AS crypto::Sealed`**

Seals `plaintext` and additionally authenticates (but does not encrypt) `aad`.
The same `aad` must be supplied to `crypto::chacha20Poly1305Open` for verification
to succeed. [[src/builtins/crypto.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `key` | `List OF Byte` | The 256-bit ChaCha20 key. Must be exactly 32 bytes. This value is secret. [[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Seal]] |
| `nonce` | `List OF Byte` | The 96-bit RFC 8439 nonce. Must be exactly 12 bytes and unique for every message encrypted under a given key. Not secret; normally transmitted with the ciphertext. [[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Seal]] |
| `plaintext` | `List OF Byte` | The message bytes to encrypt. May be empty. |
| `aad` | `List OF Byte` | Optional additional authenticated data: authenticated but not encrypted. Defaults to the empty list. The same `aad` must be passed to `crypto::chacha20Poly1305Open`. [[src/builtins/crypto.rs:default_argument_padding]] |

## Return value

| Type | Description |
| --- | --- |
| `crypto::Sealed` | A record with two fields: `ciphertext` (a `List OF Byte` the same length as `plaintext`) and `tag` (the 16-byte Poly1305 authentication tag). [[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Seal]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `key` is not exactly 32 bytes, or `nonce` is not exactly 12 bytes. [[src/builtins/crypto_package.mfb:__crypto_chacha20Poly1305Seal]] |

## Examples

Seal a message with a fresh random nonce:

```
IMPORT crypto
IMPORT encoding
IMPORT strings

LET key AS List OF Byte = crypto::randomBytes(32)
LET nonce AS List OF Byte = crypto::randomBytes(12)
LET plaintext AS List OF Byte = strings::toBytes("attack at dawn")
LET box AS crypto::Sealed = crypto::chacha20Poly1305Seal(key, nonce, plaintext)

PRINT encoding::hexEncode(box.ciphertext)
PRINT encoding::hexEncode(box.tag)
```

Seal with additional authenticated data (a header), then open it:

```
IMPORT crypto
IMPORT strings

LET key AS List OF Byte = crypto::randomBytes(32)
LET nonce AS List OF Byte = crypto::randomBytes(12)
LET plaintext AS List OF Byte = strings::toBytes("attack at dawn")
LET header AS List OF Byte = strings::toBytes("v1")
LET box AS crypto::Sealed = crypto::chacha20Poly1305Seal(key, nonce, plaintext, header)
LET clear AS List OF Byte = crypto::chacha20Poly1305Open(key, nonce, box.ciphertext, box.tag, header)
```

## See also

- `mfb man crypto chacha20Poly1305Open`
- `mfb man crypto aes256GcmSeal`
- `mfb man crypto randomBytes`
- `mfb man encoding hexEncode`

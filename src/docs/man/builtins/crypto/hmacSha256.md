# hmacSha256

Compute the HMAC-SHA-256 message authentication code (RFC 2104) of a message under a key.

## Synopsis

```
crypto::hmacSha256(key AS List OF Byte, data AS List OF Byte) AS List OF Byte
crypto::hmacSha256(key AS List OF Byte, data AS String) AS List OF Byte
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

`crypto::hmacSha256` computes the keyed-hash message authentication code of
`data` under `key`, using SHA-256 as the underlying hash, as specified by
RFC 2104. It returns a fixed 32-byte (256-bit) MAC as a `List OF Byte`.
[[src/builtins/crypto.rs:call_return_type_name]]

Keys of any length are accepted. Per RFC 2104, a key longer than the 64-byte
SHA-256 block size is first hashed down to 32 bytes, and any key shorter than
the block size is right-padded with zero bytes to 64 bytes before the inner and
outer passes. [[src/builtins/crypto_hash.mfb:__crypto_hmacSha256_bytes]]

The MAC is a deterministic function of `key` and `data` alone: the same key and
message always produce the same 32 bytes, with no salting or randomness. The
function is **total** — every combination of inputs, including empty key and
empty message, yields a MAC and it never raises an error.

The MAC is a portable software core computed over the `bits` package, so its
output is **byte-identical on every target** (macOS/Linux, aarch64/x86-64) and
uses no platform crypto library. [[src/builtins/crypto.rs:implementation_name]]

A MAC is raw binary, not text. To display or store it, stringify it with the
`encoding` package — `encoding::hexEncode` for lowercase hex or
`encoding::base64Encode` for Base64. To compare a received MAC against a
computed one, use `crypto::constantTimeEqual` so the comparison does not leak
timing information.

## Overloads

**`crypto::hmacSha256(key AS List OF Byte, data AS List OF Byte) AS List OF Byte`**

Authenticates the raw bytes of `data` exactly as given.

**`crypto::hmacSha256(key AS List OF Byte, data AS String) AS List OF Byte`**

Authenticates the UTF-8 encoding of the string. It is equivalent to converting
the string to its UTF-8 bytes and authenticating those; the concrete `data` type
selects the `_text` implementation body.
[[src/builtins/crypto.rs:implementation_name]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `key` | `List OF Byte` | The secret HMAC key. Any length is accepted, including the empty list. |
| `data` | `List OF Byte` | The message bytes to authenticate. Any length is accepted. |
| `data` | `String` | A message string whose UTF-8 bytes are authenticated. |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The 32-byte HMAC-SHA-256 code of `data` under `key`. Always exactly 32 bytes regardless of input length. [[src/builtins/crypto.rs:call_return_type_name]] |

## Errors

No errors.

## Type checking

The first argument (`key`) must be a `List OF Byte`. The second argument (`data`)
must be either a `List OF Byte` or a `String`; no other type resolves. Exactly
two arguments are required. The return type is always `List OF Byte`.
[[src/builtins/crypto.rs:resolve_call]] [[src/builtins/crypto.rs:arity]]

## Examples

Authenticate a message and print the MAC as hex:

```
IMPORT crypto
IMPORT strings
IMPORT encoding
IMPORT io

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET message AS List OF Byte = strings::toBytes("attack at dawn")
  LET mac AS List OF Byte = crypto::hmacSha256(key, message)
  io::print(encoding::hexEncode(mac))
END SUB
```

Verify a received MAC in constant time:

```
IMPORT crypto
IMPORT io

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET received AS List OF Byte = crypto::hmacSha256(key, "payload")
  LET expected AS List OF Byte = crypto::hmacSha256(key, "payload")
  IF crypto::constantTimeEqual(expected, received) THEN
    io::print("authentic")
  END IF
END SUB
```

## See also

- `mfb man crypto hmacSha512`
- `mfb man crypto sha256`
- `mfb man crypto constantTimeEqual`
- `mfb man crypto pbkdf2Sha256`
- `mfb man encoding hexEncode`

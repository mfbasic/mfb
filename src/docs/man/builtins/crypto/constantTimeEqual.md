# constantTimeEqual

Compare two byte lists for equality in time that does not depend on their contents.

## Synopsis

```
crypto::constantTimeEqual(a AS List OF Byte, b AS List OF Byte) AS Boolean
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

`crypto::constantTimeEqual` reports whether the byte lists `a` and `b` are
equal, taking time that is independent of their byte contents. Unlike an
ordinary comparison it does not return early at the first differing byte: it
accumulates the difference of every byte position and only then reports the
result, so an attacker cannot learn how many leading bytes matched by measuring
how long the comparison took. This is the correct way to compare secrets such
as message authentication codes, password hashes, and tokens.
[[src/builtins/crypto_package.mfb:__crypto_constantTimeEqual]]

The comparison works by folding `a[i] XOR b[i]` into a running OR accumulator
across all `i`; the result is `TRUE` exactly when that accumulator is zero,
meaning no byte differed. Every byte position is always examined.
[[src/builtins/crypto_package.mfb:__crypto_constantTimeEqual]]

**What is and is not secret.** Only the byte contents are protected. The lengths
of the inputs are not treated as secret. A length difference is folded into the
accumulated difference rather than taken as an early-return branch, so the
comparison does not branch on length (in)equality; the per-byte loop still runs
over the shared prefix, so the running time may reveal the (min) length. When
comparing values that should be a fixed size (for example a 32-byte HMAC tag), the
byte contents of same-length inputs are what stays constant-time (bug-269 /
CRY-03). [[src/builtins/crypto_package.mfb:__crypto_constantTimeEqual]]

The function is **total** — every combination of inputs, including two empty
lists (which compare equal), yields a Boolean and it never raises an error. Its
result is a portable software computation over the `bits` package, so it behaves
identically on every target (macOS/Linux, aarch64/x86-64) and uses no platform
crypto library. [[src/builtins/crypto.rs:implementation_name]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `List OF Byte` | The first byte list. Any length is accepted, including the empty list. [[src/builtins/crypto.rs:call_param_names]] |
| `b` | `List OF Byte` | The second byte list. Any length is accepted, including the empty list. [[src/builtins/crypto.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` if `a` and `b` have the same length and every byte is equal; otherwise `FALSE`. Two empty lists return `TRUE`. [[src/builtins/crypto.rs:call_return_type_name]] |

## Errors

No errors.

## Type checking

Both arguments must be `List OF Byte`; no other type resolves. Exactly two
arguments are required. The return type is always `Boolean`.
[[src/builtins/crypto.rs:resolve_call]] [[src/builtins/crypto.rs:arity]]

## Examples

Verify a received MAC without leaking timing:

```
IMPORT crypto
IMPORT strings
IMPORT io

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET message AS List OF Byte = strings::toBytes("payload")
  LET received AS List OF Byte = crypto::hmacSha256(key, message)
  LET expected AS List OF Byte = crypto::hmacSha256(key, message)
  IF crypto::constantTimeEqual(expected, received) THEN
    io::print("authentic")
  ELSE
    io::print("tampered")
  END IF
END SUB
```

## See also

- `mfb man crypto hmacSha256`
- `mfb man crypto hmacSha512`
- `mfb man crypto sha256`
- `mfb man encoding hexEncode`

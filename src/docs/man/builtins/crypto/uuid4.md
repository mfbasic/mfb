# uuid4

Return a random RFC 4122 version-4 UUID as a canonical lowercase string.

## Synopsis

```
crypto::uuid4() AS String
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

`crypto::uuid4` returns a random version-4 UUID as a canonical lowercase
`String` in the 8-4-4-4-12 hyphenated form — for example
`"f47ac10b-58cc-4372-a567-0e02b2c3d479"`. The result is always 36 characters:
32 hexadecimal digits plus the four hyphens.

A version-4 UUID is 122 bits of randomness with the 4-bit version field fixed to
`4` and the 2-bit variant field fixed to the RFC 4122 variant, exactly as the
standard prescribes. Internally `uuid4` draws 16 random bytes, forces the version
nibble of byte 6 and the variant bits of byte 8, hex-encodes the 16 bytes, and
splits the digits into the five hyphenated groups. [[src/builtins/crypto_util.mfb:__crypto_uuid4]]

The random bytes come from the same OS CSPRNG as `crypto::randomBytes`
(`getentropy` on both macOS and Linux), so the identifiers are cryptographically
strong and effectively collision-free in practice.
[[src/target/shared/code/crypto.rs:lower_crypto_random_bytes_helper]] As with all
`crypto` random helpers the generator is **not** seedable, so each call produces a
fresh, non-reproducible value. Use `uuid4` whenever a random, unguessable
identifier is needed; for its fast, seedable, non-cryptographic counterpart see
`math::rand`, which must never be used for security-sensitive identifiers.

This function takes no arguments and, barring a platform entropy failure or an
allocation failure, always succeeds.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `uuid4` takes no parameters. [[src/builtins/crypto.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A canonical lowercase 8-4-4-4-12 version-4 UUID, 36 characters including the four hyphens. [[src/builtins/crypto.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050000` | `ErrUnknown` | The OS entropy call (`getentropy`) backing the internal `crypto::randomBytes(16)` draw fails. [[src/target/shared/code/crypto.rs:lower_crypto_random_bytes_helper]] |
| `77010001` | `ErrOutOfMemory` | An arena allocation for the random bytes or the assembled UUID `String` fails. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Generate a unique identifier:

```
IMPORT crypto
IMPORT io

SUB main()
  LET id AS String = crypto::uuid4()
  io::print(id)
END SUB
```

Each call yields a distinct value:

```
IMPORT crypto
IMPORT io

SUB main()
  LET a AS String = crypto::uuid4()
  LET b AS String = crypto::uuid4()
  io::print(toString(a <> b))
END SUB
```

## See also

- `mfb man crypto randomBytes`
- `mfb man crypto randomInt`
- `mfb man math rand`

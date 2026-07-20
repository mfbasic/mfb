# randomBytes

Return cryptographically secure random bytes drawn from the OS CSPRNG.

## Synopsis

```
crypto::randomBytes(count AS Integer) AS List OF Byte
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

`crypto::randomBytes` returns `count` fresh bytes drawn from the operating
system's cryptographically secure pseudo-random number generator (CSPRNG). The
bytes are produced by `getentropy`, the non-deprecated OS entropy source present
on both macOS and Linux (glibc and musl), so the output is suitable for keys,
nonces, salts, tokens, and any other use where unpredictability is a security
requirement. [[src/target/shared/code/crypto.rs:lower_crypto_random_bytes_helper]]

Unlike the portable software cores in this package (the hashes, HMAC, HKDF,
PBKDF2, and the AEADs), `randomBytes` is a **native runtime helper** rather than
source: it routes to `_mfb_rt_crypto_crypto_randomBytes` and reads OS entropy
directly, so its output is inherently non-reproducible and platform-provided
rather than byte-identical across targets.
[[src/builtins/crypto.rs:is_native_crypto_call]]

This generator is cryptographically secure and, by design, **not** seedable:
there is no way to fix or replay its output. That is the deliberate contrast with
`math::rand`, a fast, seedable PCG64 generator intended for simulations,
sampling, and games. `math::rand` is **not** cryptographically secure and must
never be used for keys, tokens, or nonces; `crypto::randomBytes` is the correct
source for all such material.

Each call draws fresh entropy, so results are not reproducible across runs.
`count` must be in the range `0` to `16777216` (16 MiB) inclusive: a `count` of 0
returns an empty list, while a negative `count` or one above the 16 MiB cap raises
`ErrInvalidArgument`. The upper bound sits far above any real key-material request
and rejects an absurd allocation before its size arithmetic can overflow.
Internally the fill runs in chunks of at most 256 bytes (the per-call
`getentropy` limit), transparent to the caller.
[[src/target/shared/code/crypto.rs:GETENTROPY_MAX]] [[src/target/shared/code/crypto.rs:RANDOM_BYTES_MAX_COUNT]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `count` | `Integer` | The number of random bytes to return. Must be in `0` to `16777216` (16 MiB) inclusive; `0` yields an empty list. [[src/target/shared/code/crypto.rs:RANDOM_BYTES_MAX_COUNT]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | Exactly `count` cryptographically secure random bytes. An empty list when `count` is `0`. [[src/builtins/crypto.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `count` is negative or exceeds `16777216` (16 MiB). [[src/target/shared/code/crypto.rs:RANDOM_BYTES_MAX_COUNT]] |
| `77050000` | `ErrUnknown` | The OS entropy call (`getentropy`) fails. [[src/target/shared/code/crypto.rs:lower_crypto_random_bytes_helper]] |
| `77010001` | `ErrOutOfMemory` | The arena allocation for the result bytes fails. [[src/target/shared/code/crypto.rs:lower_crypto_random_bytes_helper]] |

## Type checking

`randomBytes` takes exactly one `Integer` argument and returns `List OF Byte`; no
other arity or argument type resolves.
[[src/builtins/crypto.rs:resolve_call]] [[src/builtins/crypto.rs:arity]]

## Examples

Generate a 32-byte key and a 12-byte AEAD nonce:

```
IMPORT crypto

SUB main()
  LET key AS List OF Byte = crypto::randomBytes(32)
  LET nonce AS List OF Byte = crypto::randomBytes(12)
END SUB
```

A count of zero returns an empty list:

```
IMPORT crypto

SUB main()
  LET none AS List OF Byte = crypto::randomBytes(0)
END SUB
```

## See also

- `mfb man crypto randomInt`
- `mfb man crypto uuid4`
- `mfb man math rand`

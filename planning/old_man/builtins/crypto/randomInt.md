# randomInt

Return a cryptographically secure, uniformly distributed integer in an inclusive range.

## Synopsis

```
crypto::randomInt(min AS Integer, max AS Integer) AS Integer
```

## Package

crypto

## Imports

```
IMPORT crypto
```

`crypto` is a built-in package, so no manifest dependency is required.
[[src/codegen/registry/mod.rs:augment_project]]

## Description

`crypto::randomInt` returns a uniformly distributed random `Integer` in the
inclusive range `[min, max]`. Both endpoints are attainable, so the number of
possible results is `max - min + 1`. When `min` equals `max` the single value in
range is returned directly.
[[src/codegen/builtins/crypto/package.mfb:__crypto_randomInt]]

The randomness comes from the same OS CSPRNG as `crypto::randomBytes`
(`getentropy`): `randomInt` is source glue that draws fresh entropy through
`crypto::randomBytes` for every call, so results are cryptographically secure and,
by design, **not** seedable or reproducible across runs.
[[src/codegen/builtins/crypto/package.mfb:__crypto_rand62]]

The distribution is unbiased. Rather than reducing raw entropy modulo the range —
which skews toward smaller values when the range does not divide the entropy space
evenly — `randomInt` uses rejection sampling: it draws a uniform 62-bit value and
discards any draw at or above the largest exact multiple of the range
(`maxVal - (maxVal MOD range)`, where `maxVal` is `2^62`), guaranteeing every
value in `[min, max]` is equally likely.
[[src/codegen/builtins/crypto/package.mfb:__crypto_randomInt]]

This is the cryptographic counterpart to `math::rand`'s integer helpers, which
are fast and seedable but **not** cryptographically secure. Use
`crypto::randomInt` whenever the value must be unpredictable to an adversary.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `min` | `Integer` | The inclusive lower bound of the range. [[src/codegen/registry/mod.rs:call_param_names]] |
| `max` | `Integer` | The inclusive upper bound of the range. Must be greater than or equal to `min`. [[src/codegen/builtins/crypto/package.mfb:__crypto_randomInt]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | A uniformly distributed value `x` with `min <= x <= max`. Returns `min` when `min` equals `max`. [[src/codegen/builtins/crypto/mod.rs:CRYPTO]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `min` is greater than `max`, or the range `max - min + 1` overflows a non-negative `Integer` and is too large to sample. [[src/codegen/builtins/crypto/package.mfb:__crypto_randomInt]] |

## Type checking

`randomInt` takes exactly two `Integer` arguments and returns `Integer`; no other
arity or argument type resolves.
[[src/codegen/builtins/crypto/mod.rs:CRYPTO]] [[src/codegen/builtins/crypto/mod.rs:CRYPTO]]

## Examples

Roll a fair six-sided die:

```
IMPORT crypto

SUB main()
  LET roll AS Integer = crypto::randomInt(1, 6)
END SUB
```

A single-value range always returns that value:

```
IMPORT crypto

SUB main()
  LET x AS Integer = crypto::randomInt(42, 42)
END SUB
```

## See also

- `mfb man crypto randomBytes`
- `mfb man crypto uuid4`
- `mfb man math rand`

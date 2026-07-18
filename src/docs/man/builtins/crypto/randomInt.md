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
[[src/builtins/crypto.rs:augmented_project]]

## Description

`crypto::randomInt` returns a uniformly distributed random `Integer` in the
inclusive range `[min, max]`. Both endpoints are attainable, so the number of
possible results is `max - min + 1`. When `min` equals `max` the single value in
range is returned directly.
[[src/builtins/crypto_package.mfb:__crypto_randomInt]]

The randomness comes from the same OS CSPRNG as `crypto::randomBytes`
(`getentropy`): `randomInt` is source glue that draws fresh entropy through
`crypto::randomBytes` for every call, so results are cryptographically secure and,
by design, **not** seedable or reproducible across runs.
[[src/builtins/crypto_package.mfb:__crypto_rand62]]

The distribution is unbiased. Rather than reducing raw entropy modulo the range —
which skews toward smaller values when the range does not divide the entropy space
evenly — `randomInt` uses rejection sampling: it draws a uniform 62-bit value and
discards any draw at or above the largest exact multiple of the range
(`maxVal - (maxVal MOD range)`, where `maxVal` is `2^62`), guaranteeing every
value in `[min, max]` is equally likely.
[[src/builtins/crypto_package.mfb:__crypto_randomInt]]

This is the cryptographic counterpart to `math::rand`'s integer helpers, which
are fast and seedable but **not** cryptographically secure. Use
`crypto::randomInt` whenever the value must be unpredictable to an adversary.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `min` | `Integer` | The inclusive lower bound of the range. [[src/builtins/crypto.rs:call_param_names]] |
| `max` | `Integer` | The inclusive upper bound of the range. Must be greater than or equal to `min`. [[src/builtins/crypto_package.mfb:__crypto_randomInt]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | A uniformly distributed value `x` with `min <= x <= max`. Returns `min` when `min` equals `max`. [[src/builtins/crypto.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `min` is greater than `max`, or the range `max - min + 1` overflows a non-negative `Integer` and is too large to sample. [[src/builtins/crypto_package.mfb:__crypto_randomInt]] |

## Type checking

`randomInt` takes exactly two `Integer` arguments and returns `Integer`; no other
arity or argument type resolves.
[[src/builtins/crypto.rs:resolve_call]] [[src/builtins/crypto.rs:arity]]

## Examples

Roll a fair six-sided die:

```
IMPORT crypto

LET roll AS Integer = crypto::randomInt(1, 6)
```

A single-value range always returns that value:

```
IMPORT crypto

LET x AS Integer = crypto::randomInt(42, 42)
```

## See also

- `mfb man crypto randomBytes`
- `mfb man crypto uuid4`
- `mfb man math rand`

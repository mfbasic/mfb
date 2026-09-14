# Arbitrary-precision integers (big)

The `big` package provides `big::Int`, a signed integer with no fixed size, and the
arithmetic, text and division members over it. Called with the `big::` qualifier;
`IMPORT big` needs no manifest dependency. [[src/codegen/builtins/big/mod.rs:register]]

This topic specifies the *model behind* the package: the value's representation and
canonical form, why the language operators do not apply, which members can fail, the
division and remainder rule, and the timing property. The per-function API — signatures,
parameters, errors — is owned by `./mfb man big`.

## The value

`big::Int` is an ordinary exported value record with two fields:

| Field | Type | Meaning |
|-------|------|---------|
| `magnitude` | `List OF Byte` | the absolute value, least significant byte first |
| `negative` | `Boolean` | `TRUE` when the value is below zero |

The value is `magnitude` read as an unsigned little-endian number, negated when
`negative` is `TRUE`. Field order is part of the contract: every member reads and builds a
`big::Int` at the slots the declaration fixes. [[src/codegen/builtins/big/mod.rs:register]]

A `big::Int` is a **value**, not a handle. Assigning it or passing it to a function makes an
independent copy; there is nothing to open and no `close`. A `big::Int` declared with `MUT`
and no initializer holds zero: a `List` field defaults to the empty list and a `Boolean` to
`FALSE`, and `{[], FALSE}` is canonical zero. (An immutable `LET` must have an initializer,
as for every type.) [[src/ir/verify/resources.rs:is_defaultable]]

### Canonical form

Every member returns its result in **canonical form**:

- `magnitude` has no zero bytes at its most significant end;
- `magnitude` is empty exactly when the value is zero;
- `negative` is `FALSE` for zero, so there is no negative zero.

A record can also be built by hand (`big::Int[[7, 0], FALSE]`), so canonical form cannot be
enforced on input. The package therefore reads every argument **totally**: trailing zero
bytes are ignored and a zero magnitude reads as non-negative, so a non-canonical record is
accepted everywhere as the number it spells. Normalization happens in exactly one place, on
the way out of every member, which is what makes canonical form a property of the package
rather than of each member. [[src/codegen/builtins/big/gen_big.rs:emit_load_int]]
[[src/codegen/builtins/big/gen_big.rs:emit_build_int]]

`big::Endian` (`Little`, `Big`) selects the byte order at the `List OF Byte` boundary
(`big::fromBytes`, `big::toBytes`); the stored magnitude is always least significant byte
first.

## Operators, keys and elements

The language operators do not apply to a `big::Int`, and cannot be made to:

- Equality and inequality (`=`, `<>`) are rejected because a record is comparable only when every field is, and a `List` field is not. [[src/ir/verify/values.rs:is_comparable_seen]]
- Ordering and arithmetic (`<`, `>`, `+`, `-`, `*`, `/`) are rejected because records are never orderable and have no arithmetic.
- For the same comparability reason a `big::Int` cannot be a `Map` key or a `Set` element.

The members replace them: `big::compare` and `big::equals` for ordering and equality,
`big::isZero` and `big::sign` for the common tests, and `big::add`, `big::subtract`,
`big::multiply`, `big::divide` and `big::remainder` for arithmetic. To key a map by a
`big::Int`, key it by `big::toString` of the value; the text is canonical, so equal values
give equal keys.

## Failure

The arithmetic cannot overflow — a result grows to hold whatever value it is — so most
members are **total**: `fromInteger`, `fromBytes`, `toBytes`, `compare`, `equals`,
`isZero`, `sign`, `abs`, `negate`, `add`, `subtract`, `multiply`, `sum`, `product`,
`bitLength`, `toString` and `gcd` declare no error.

The fallible members, and the one condition each checks:

| Member | Raises | When |
|--------|--------|------|
| `toInteger` | `ErrOverflow` | the value is outside the `Integer` range |
| `shiftLeft`, `shiftRight` | `ErrInvalidArgument` | a negative `count` |
| `testBit` | `ErrInvalidArgument` | a negative `index` |
| `parse` | `ErrInvalidFormat` | the text is not an optional `-` and digits below the radix |
| `parse`, `toRadixString` | `ErrInvalidArgument` | a radix outside 2 to 36 |
| `divide`, `remainder`, `divMod` | `ErrInvalidArgument` | a zero divisor |
| `pow`, `factorial` | `ErrInvalidArgument` | a negative exponent or `n` |
| `modPow` | `ErrInvalidArgument` | a zero modulus or a negative exponent |

As for every built-in, a request for more memory than the program can have raises
`ErrOutOfMemory`; it is not listed per member.

## Division and remainder

Division **truncates toward zero** and the remainder **takes the sign of the dividend** —
the rule `Integer`'s own `/` and `MOD` follow (`-7 / 2` is `-3`, `-7 MOD 2` is `-1`,
`7 MOD -2` is `1`). So for every `a` and every non-zero `b`,
`a = b * divide(a, b) + remainder(a, b)` with `|remainder(a, b)| < |b|`, and for values that
fit an `Integer` the members agree with the operators. `big::divMod` returns both from one
division as a `big::DivResult` record (`quotient`, `remainder`).

`big::gcd` is never negative and is defined for every pair, including `gcd(0, 0) = 0`.
`big::modPow(base, exponent, modulus)` equals `big::remainder(big::pow(base, exponent),
modulus)` — a non-zero result takes the sign that power would have — but reduces after every
step, so a huge `exponent` stays practical.

## Timing

**Nothing in the big package runs in constant time.** Comparison stops at the first byte that
differs, multiplication and division take time that grows with the operands' sizes, and
`big::modPow`'s running time and memory access follow the bits of its exponent. Timing a
call therefore reveals information about the values involved, so a `big::Int` must never
hold a secret that an observer must not learn. Use `crypto::` for anything cryptographic: its
key, signature and field operations are built from constant-time primitives for exactly this
reason, and `crypto::constantTimeEqual` compares secret bytes. The one place `crypto`
accepts a `big::Int` is `crypto::randomInt`'s range overload, which only draws public-range
random values (see `./mfb man crypto randomInt`).

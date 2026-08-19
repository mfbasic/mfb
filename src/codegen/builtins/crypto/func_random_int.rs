//! `crypto::randomInt` — descriptor entry + authored docs.
//!
//! A source-glue member: an unbiased, CSPRNG-backed integer in an inclusive range,
//! drawing fresh entropy through `crypto::randomBytes` per call. Its
//! `Body::Rewrite("__crypto_randomInt")` repoints the citation at the `package.mfb`
//! helper.

use super::{Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str =
    r#"Return a cryptographically secure, uniformly distributed integer in an inclusive range."#;
const DESC: &str = r#"`crypto::randomInt` returns a uniformly distributed random `Integer` in the
inclusive range `[min, max]`. Both endpoints are attainable, so the number of
possible results is `max - min + 1`. When `min` equals `max` the single value in
range is returned directly.

**Range and errors.** `max` must be `>= min`, otherwise `ErrInvalidArgument` is
raised. Because the count of outcomes is `max - min + 1`, a span so large that
`max - min` overflows a signed 64-bit `Integer` is also rejected with
`ErrInvalidArgument`. Every other combination is valid, including negative bounds
and the full width of the non-negative `Integer` range.

**Unbiased sampling.** The distribution is exactly uniform. Rather than reducing
raw entropy modulo the range — which skews toward smaller values when the range
does not divide the entropy space evenly — `randomInt` uses rejection sampling.
For a range up to `2^62` it draws a uniform 62-bit value (`maxVal = 2^62 =
4611686018427387904`) and discards any draw at or above the largest exact
multiple of the range, `maxVal - (maxVal MOD range)`; the accepted value is then
reduced modulo the range. For a wider range — one greater than `2^62`, which can
occur only when `max - min` falls in the band `(2^62, 2^63 - 1]` — it draws a
uniform 63-bit value instead and rejects any draw `>= range`, with no modulo
needed. Either way every value in `[min, max]` is equally likely.

**Security caveats.** The entropy comes from `crypto::randomBytes` (the OS
CSPRNG), so results are cryptographically secure and, by design, **not** seedable
or reproducible across runs. This is the cryptographic counterpart to
`math::rand`'s integer helpers, which are fast and seedable but **not**
cryptographically secure. Use `crypto::randomInt` whenever the value must be
unpredictable to an adversary.

**Implementation.** `randomInt` is portable MFBASIC software layered over
`crypto::randomBytes`: it draws fresh entropy through `crypto::randomBytes` for
every call (in 8-byte draws, one per rejection-sampling attempt). Its logic is
byte-identical on every target (macOS/Linux, aarch64/x86-64); only the entropy,
and therefore the values, differs. Because the entropy is drawn through
`crypto::randomBytes`, an OS entropy failure or allocation failure there
propagates out as `ErrUnknown` or `ErrOutOfMemory`."#;
const EX: &str = r#"Roll a fair six-sided die:

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
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "randomInt",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "min",
                    desc: "Inclusive lower bound. May be negative.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "max",
                    desc: "Inclusive upper bound. Must be `>= min`, and `max - min` \
                           must fit in a signed 64-bit `Integer`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec!["ErrInvalidArgument", "ErrUnknown", "ErrOutOfMemory"],
            body: Body::Rewrite("__crypto_randomInt"),
        }],
    });
}

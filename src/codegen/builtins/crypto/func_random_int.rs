//! `crypto::randomInt` — descriptor entry + authored docs.
//!
//! A source-glue member: an unbiased, CSPRNG-backed integer in an inclusive range,
//! drawing fresh entropy through `crypto::randomBytes` per call. Its
//! `Body::Rewrite("__crypto_randomInt")` repoints the citation at the `package.mfb`
//! helper. Docs migrated from `src/docs/man/builtins/crypto/randomInt.md`.

use super::{Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction};

const INTRO: &str =
    r#"Return a cryptographically secure, uniformly distributed integer in an inclusive range."#;
const DESC: &str = r#"`crypto::randomInt` returns a uniformly distributed random `Integer` in the
inclusive range `[min, max]`. Both endpoints are attainable, so the number of
possible results is `max - min + 1`. When `min` equals `max` the single value in
range is returned directly.

The randomness comes from the same OS CSPRNG as `crypto::randomBytes`
(`getentropy`): `randomInt` is source glue that draws fresh entropy through
`crypto::randomBytes` for every call, so results are cryptographically secure and,
by design, **not** seedable or reproducible across runs.

The distribution is unbiased. Rather than reducing raw entropy modulo the range —
which skews toward smaller values when the range does not divide the entropy space
evenly — `randomInt` uses rejection sampling: it draws a uniform 62-bit value and
discards any draw at or above the largest exact multiple of the range
(`maxVal - (maxVal MOD range)`, where `maxVal` is `2^62`), guaranteeing every
value in `[min, max]` is equally likely.

This is the cryptographic counterpart to `math::rand`'s integer helpers, which
are fast and seedable but **not** cryptographically secure. Use
`crypto::randomInt` whenever the value must be unpredictable to an adversary."#;
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

pub(super) fn register(pkg: &mut super::RegistryPackage) {
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
                    desc: "Inclusive lower bound.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "max",
                    desc: "Inclusive upper bound; must be >= min.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec!["ErrInvalidArgument"],
            body: Body::Rewrite("__crypto_randomInt"),
        }],
    });
}

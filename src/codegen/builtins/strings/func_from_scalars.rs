//! `strings::fromScalars` — scalar-seam member (`Body::Rewrite`).
//!
//! Backed by the injected scalar-seam chunk (`helper_scalar_seam.rs`, gated `WhenUsed`): a call
//! rewrites to the internal `__strings_fromScalars` FUNC through the registry's
//! `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Build a string from a list of Unicode scalar values."#;

const DESC: &str = r#"`strings::fromScalars` encodes a `List OF Scalar` into a `String` by
concatenating the UTF-8 encoding of each element, in order.

It is the inverse of `strings::toScalars`: `fromScalars(toScalars(s))` equals `s`
for every `String s`. Encoding always succeeds because a `Scalar` is by
construction a valid, non-surrogate Unicode code point, so there is no
ill-formed input to reject.

Each element contributes one to four bytes depending on its code point, so the
byte length of the result is generally larger than the element count, while
`len` of the result equals the element count exactly. The empty list yields the
empty string.

The input list is not modified; the returned `String` is a fresh owned value."#;

const EX: &str = r#"Build a string from scalar literals:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET chars AS List OF Scalar = [`h`, `i`, `!`]
  io::print(strings::fromScalars(chars))
  RETURN 0
END FUNC
```

Round-trip a string through its scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET original AS String = "héllo中😀"
  LET rebuilt AS String = strings::fromScalars(strings::toScalars(original))
  io::print(toString(rebuilt = original))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fromScalars",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "scalars",
                desc: "The scalars to encode, in order. Any `List OF Scalar` is accepted, including the empty list.",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::named("Scalar")),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::Rewrite("__strings_fromScalars"),
        }],
    });
}

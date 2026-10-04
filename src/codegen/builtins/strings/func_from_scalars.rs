//! `strings::fromScalars` — lowered natively (`gen_scalar_seam::lower_from_scalars`,
//! bug-772): one pass to sum the UTF-8 widths, one exactly-sized `String`, one pass
//! to encode.

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
byte length of the result (`strings::byteLen`) is never smaller than the element
count — equal only when every scalar is ASCII — while
`len` of the result equals the element count exactly. The empty list yields the
empty string.

The input list is not modified; the returned `String` is its own value."#;

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
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_inline(super::gen_scalar_seam::lower_from_scalars),
        }],
    });
}

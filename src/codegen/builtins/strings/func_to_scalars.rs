//! `strings::toScalars` — scalar-seam member (`Body::Rewrite`).
//!
//! Backed by the injected scalar-seam chunk (`helper_scalar_seam.rs`, gated `WhenUsed`): a call
//! rewrites to the internal `__strings_toScalars` FUNC through the registry's
//! `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Decode a string into its Unicode scalar values."#;

const DESC: &str = r#"`strings::toScalars` decodes `value` into its Unicode scalar values and returns
them, in order, as a `List OF Scalar`. It walks the UTF-8 once, yielding one
element per code point.

Each element is one `Scalar` — a 32-bit Unicode scalar value — not a grapheme
cluster. A base letter followed by a combining mark is two separate elements,
while an astral character such as an emoji is a single element. The element count
therefore equals `len(value)` and is generally smaller than
`strings::byteLen(value)`. Use `strings::graphemes` when user-perceived
characters are what matters.

This is the entry point for walking a string one scalar at a time: compare each
`Scalar`, `MATCH` on it, or classify it with `strings::isLetter` and its
siblings, then rebuild a `String` with `strings::fromScalars`. The round trip is
exact — `fromScalars(toScalars(s))` equals `s` for every `String s` — because
every `String` is well-formed UTF-8 by construction, so decoding cannot fail.

The scalars appear in the same left-to-right order as in `value`. The empty
string yields the empty list. `value` is not mutated; the returned list is a
fresh owned value.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"Count the scalars in a string with an astral character:

```
IMPORT io
IMPORT strings
IMPORT collections

FUNC main() AS Integer
  LET scalars AS List OF Scalar = strings::toScalars("a中😀")
  io::print(toString(len(scalars)))
  io::print(toString(collections::get(scalars, 0) = `a`))
  RETURN 0
END FUNC
```

Keep only the letters and digits, then rebuild the string:

```
IMPORT io
IMPORT strings
IMPORT collections

FUNC main() AS Integer
  MUT kept AS List OF Scalar = []
  FOR EACH sc IN strings::toScalars("a1 b2! c3")
    IF strings::isLetter(sc) OR strings::isDigit(sc) THEN
      kept = collections::append(kept, sc)
    END IF
  NEXT
  io::print(strings::fromScalars(kept))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toScalars",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string to decode. Any `String` is accepted, including the empty string.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::named("Scalar")),
            errors: vec![],
            body: Body::Rewrite("__strings_toScalars"),
        }],
    });
}

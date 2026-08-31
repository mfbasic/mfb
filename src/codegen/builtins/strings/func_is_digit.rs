//! `strings::isDigit` — scalar-seam member (`Body::Rewrite`).
//!
//! Backed by the injected scalar-seam chunk (`helper_scalar_seam.rs`, gated `WhenUsed`): a call
//! rewrites to the internal `__strings_isDigit` FUNC through the registry's
//! `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether a Unicode scalar is a decimal digit."#;

const DESC: &str = r#"`strings::isDigit` returns `TRUE` when `scalar` has the Unicode general category
`Nd` (decimal number) and `FALSE` otherwise.

The test is exactly `Nd`, no wider. It therefore accepts ASCII `0`–`9` and the
decimal digits of other scripts, such as the Arabic-Indic and Devanagari digits,
but it rejects other numeric scalars whose category is `Nl` (letter number, for
example Roman numerals) or `No` (other number, for example superscripts and
fractions). A scalar that "looks numeric" is not necessarily a digit by this
definition.

Classification follows the Unicode general categories,
and is deterministic and locale-independent. The function is total: it returns a
`Boolean` for every `Scalar` and never fails.

`isDigit` classifies a *single* scalar. To ask a question about a whole string,
walk it with `strings::toScalars` and fold the results yourself; that decision is
deliberately left to the caller. Note also that a digit test is not a number
parser — use `toInt` or `toFloat` to convert text to a number."#;

const EX: &str = r#"Classify individual scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::isDigit(`7`)))
  io::print(toString(strings::isDigit(`x`)))
  RETURN 0
END FUNC
```

Count the digits in a string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT digits AS Integer = 0
  FOR EACH sc IN strings::toScalars("a1 b2! c3")
    IF strings::isDigit(sc) THEN
      digits = digits + 1
    END IF
  NEXT
  io::print(toString(digits))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isDigit",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "scalar",
                desc: "The Unicode scalar to classify. Any `Scalar` is accepted.",
                aliases: &[],
                ty: ParameterType::named("Scalar"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::Rewrite("__strings_isDigit"),
        }],
    });
}

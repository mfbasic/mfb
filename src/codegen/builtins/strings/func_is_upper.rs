//! `strings::isUpper` — scalar-seam member (`Body::Rewrite`).
//!
//! Backed by the injected scalar-seam chunk (`helper_scalar_seam.rs`, gated `WhenUsed`): a call
//! rewrites to the internal `__strings_isUpper` FUNC through the registry's
//! `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether a Unicode scalar is an uppercase letter."#;

const DESC: &str = r#"`strings::isUpper` returns `TRUE` when `scalar` has the Unicode general category
`Lu` (uppercase letter) and `FALSE` otherwise.

The test is exactly `Lu`, no wider. Titlecase letters (category `Lt`, such as the
digraph `ǅ`) are **not** reported as uppercase, and neither are uncased letters,
digits, punctuation, or symbols. `isUpper` is a category test, not a
"has-no-lowercase-mapping" test.

Classification reads the Unicode general-category table embedded in the compiler,
so it covers the whole code-point space rather than just ASCII, and is
deterministic and locale-independent. The function is total: it returns a
`Boolean` for every `Scalar` and never fails.

`isUpper` classifies a *single* scalar. To ask a question about a whole string,
walk it with `strings::toScalars` and fold the results yourself. To change case
rather than test it, use `strings::upper`; for caseless comparison, use
`strings::caseFold`."#;

const EX: &str = r#"Classify individual scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::isUpper(`Q`)))
  io::print(toString(strings::isUpper(`q`)))
  io::print(toString(strings::isUpper(`7`)))
  RETURN 0
END FUNC
```

Count the uppercase letters in a string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT caps AS Integer = 0
  FOR EACH sc IN strings::toScalars("MFBasic")
    IF strings::isUpper(sc) THEN
      caps = caps + 1
    END IF
  NEXT
  io::print(toString(caps))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isUpper",
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
            body: Body::Rewrite("__strings_isUpper"),
        }],
    });
}

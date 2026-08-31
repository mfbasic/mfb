//! `strings::isLower` — scalar-seam member (`Body::Rewrite`).
//!
//! Backed by the injected scalar-seam chunk (`helper_scalar_seam.rs`, gated `WhenUsed`): a call
//! rewrites to the internal `__strings_isLower` FUNC through the registry's
//! `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether a Unicode scalar is a lowercase letter."#;

const DESC: &str = r#"`strings::isLower` returns `TRUE` when `scalar` has the Unicode general category
`Ll` (lowercase letter) and `FALSE` otherwise.

The test is exactly `Ll`, no wider. Modifier letters (category `Lm`) and other
letters (`Lo`, which covers uncased scripts such as Han and Arabic) are **not**
reported as lowercase, and neither are digits, punctuation, or symbols.
`isLower` is a category test, not a "has-no-uppercase-mapping" test.

Classification follows the Unicode general categories,
so it covers the whole code-point space rather than just ASCII, and is
deterministic and locale-independent. The function is total: it returns a
`Boolean` for every `Scalar` and never fails.

`isLower` classifies a *single* scalar. To ask a question about a whole string,
walk it with `strings::toScalars` and fold the results yourself. To change case
rather than test it, use `strings::lower`; for caseless comparison, use
`strings::caseFold`."#;

const EX: &str = r#"Classify individual scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::isLower(`q`)))
  io::print(toString(strings::isLower(`Q`)))
  io::print(toString(strings::isLower(`中`)))
  RETURN 0
END FUNC
```

Count the lowercase letters in a string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT small AS Integer = 0
  FOR EACH sc IN strings::toScalars("MFBasic")
    IF strings::isLower(sc) THEN
      small = small + 1
    END IF
  NEXT
  io::print(toString(small))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isLower",
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
            body: Body::Rewrite("__strings_isLower"),
        }],
    });
}

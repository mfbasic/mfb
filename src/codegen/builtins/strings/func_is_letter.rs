//! `strings::isLetter` — scalar-seam member (`Body::Rewrite`).
//!
//! Backed by the injected scalar-seam chunk (`helper_scalar_seam.rs`, gated `WhenUsed`): a call
//! rewrites to the internal `__strings_isLetter` FUNC through the registry's
//! `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether a Unicode scalar is a letter."#;

const DESC: &str = r#"`strings::isLetter` returns `TRUE` when `scalar` is a Unicode letter and `FALSE`
otherwise. A scalar counts as a letter when its Unicode general category is one
of `Lu` (uppercase letter), `Ll` (lowercase letter), `Lt` (titlecase letter),
`Lm` (modifier letter), or `Lo` (other letter) — that is, any `L*` category.

Classification follows the Unicode general categories,
so it covers the whole code-point space rather than just ASCII: `中` and `é` are
letters, while `5`, `-`, and a space are not. The test is deterministic and
locale-independent, with no language-specific tailoring.

The function is total: it returns a `Boolean` for every `Scalar` and never fails.

`isLetter` classifies a *single* scalar. To ask a question about a whole string,
walk it with `strings::toScalars` and fold the results yourself; that decision is
deliberately left to the caller, since "is this string all letters" has several
reasonable definitions."#;

const EX: &str = r#"Classify individual scalars, including non-ASCII ones:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::isLetter(`A`)))
  io::print(toString(strings::isLetter(`中`)))
  io::print(toString(strings::isLetter(`5`)))
  RETURN 0
END FUNC
```

Fold the predicate over a whole string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT allLetters AS Boolean = TRUE
  FOR EACH sc IN strings::toScalars("héllo")
    IF NOT strings::isLetter(sc) THEN
      allLetters = FALSE
    END IF
  NEXT
  io::print(toString(allLetters))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isLetter",
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
            body: Body::Rewrite("__strings_isLetter"),
        }],
    });
}

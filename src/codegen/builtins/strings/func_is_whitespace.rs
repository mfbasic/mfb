//! `strings::isWhitespace` — scalar-seam member (`Body::Rewrite`).
//!
//! Backed by the injected scalar-seam chunk (`helper_scalar_seam.rs`, gated `WhenUsed`): a call
//! rewrites to the internal `__strings_isWhitespace` FUNC through the registry's
//! `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether a Unicode scalar is whitespace."#;

const DESC: &str = r#"`strings::isWhitespace` returns `TRUE` when `scalar` is a Unicode whitespace
scalar and `FALSE` otherwise. The set is defined as the union of three rules:

- any scalar whose Unicode general category is `Zs` (space separator), `Zl`
  (line separator), or `Zp` (paragraph separator) — this covers `U+0020`,
  `U+00A0`, `U+1680`, `U+2000`–`U+200A`, `U+2028`, `U+2029`, `U+202F`, `U+205F`,
  and `U+3000`;
- the C0 controls `U+0009` through `U+000D` — tab, line feed, vertical tab, form
  feed, and carriage return;
- `U+0085` NEXT LINE.

Whitespace is thus *not* a single general category: the separator categories
alone omit tab and newline, which is why the control range and `U+0085` are added
explicitly. The resulting set is exactly the Unicode `White_Space` property, and
it matches the set `strings::trim`, `strings::trimStart`, and `strings::trimEnd`
remove.

Classification is deterministic and locale-independent. The function is total: it
returns a `Boolean` for every `Scalar` and never fails.

`isWhitespace` classifies a *single* scalar. To ask a question about a whole
string, walk it with `strings::toScalars` and fold the results yourself."#;

const EX: &str = r#"Classify individual scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::isWhitespace(`\t`)))
  io::print(toString(strings::isWhitespace(` `)))
  io::print(toString(strings::isWhitespace(`x`)))
  RETURN 0
END FUNC
```

Test whether a string is entirely blank:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT blank AS Boolean = TRUE
  FOR EACH sc IN strings::toScalars("  \t ")
    IF NOT strings::isWhitespace(sc) THEN
      blank = FALSE
    END IF
  NEXT
  io::print(toString(blank))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isWhitespace",
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
            body: Body::Rewrite("__strings_isWhitespace"),
        }],
    });
}

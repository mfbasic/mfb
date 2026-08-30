//! `strings.caseFold` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_case_map;
use crate::codegen::builtins::strings::UnicodeCaseMap;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Case-fold a string to a canonical caseless form for comparison."#;

const DESC: &str = r#"`strings::caseFold` returns a new `String` produced by applying Unicode full case
folding to `value`. Folding maps scalars to a canonical caseless form so that two
strings differing only in case become equal once both are folded. It is the
intended basis for caseless matching, in preference to uppercasing or lowercasing
both operands.

Folding is applied per Unicode scalar value across the whole string, using the
case-folding table embedded in the runtime. Scalars with no folded form — digits,
punctuation, and symbols — are copied through unchanged. Folding is *full*: one
scalar may fold to several, so `"Straße"` folds to `"strasse"` and the result can
be longer than the input. Never assume `len` is preserved across a fold.

Folding is not lowercasing. It is designed for comparison rather than display,
and it also collapses distinctions that lowercasing preserves — `U+212A` KELVIN
SIGN folds to plain `k`. Do not present a folded string to a user; keep the
original for display and use the folded form only as a comparison key.

Folding does not normalize. Strings that differ in Unicode normalization form can
still differ after folding, so apply `strings::normalizeNfc` first when
normalization-insensitive matching is required. The mapping is deterministic and
locale-independent, with no language-specific tailoring. `value` is not mutated;
the result is a new owned `String`.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is transformed as above, but **attributes are
dropped** — the mapping changes the scalar count within a span, so the overlay
cannot be remapped."#;

const EX: &str = r#"Compare two strings without regard to case:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET same AS Boolean = strings::caseFold("HELLO") = strings::caseFold("hello")
  io::print(toString(same))
  RETURN 0
END FUNC
```

Folding can change the length of the string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET german AS String = "Straße"
  io::print(strings::caseFold(german))
  RETURN 0
END FUNC
```

Normalize first when the inputs may differ in composition:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET a AS String = strings::caseFold(strings::normalizeNfc("CAFÉ"))
  LET b AS String = strings::caseFold(strings::normalizeNfc("café"))
  io::print(toString(a = b))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if let Some(value) = builder.static_strings_package_string("strings.caseFold", args)? {
        let register = builder.load_string_constant(&value)?;
        return Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(register.render()),
            text: "strings.caseFold".to_string(),
        });
    }
    if args.len() != 1 {
        return Err("strings.caseFold: no native lowering for these arguments".to_string());
    }
    gen_case_map::lower_strings_case_map(builder, &args[0], UnicodeCaseMap::CaseFold)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "caseFold",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string to fold. Any `String` is accepted, including the empty string.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}

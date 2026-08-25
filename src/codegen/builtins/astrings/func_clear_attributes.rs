//! `astrings::clearAttributes` — Tier-C mutation member (`Body::Rewrite`),
//! overloaded on arity.
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): the whole form (1 arg)
//! rewrites to `__astrings_clearAttributes`, the ranged form (3 args) to
//! `__astrings_clearAttributesRange`. The registry's overload-aware `rewrite_target`
//! selects the body by argument count (replacing the legacy per-package
//! `implementation_name` arity branch).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Clear all attributes, everywhere or over an inclusive scalar range."#;

const DESC: &str = r#"`clearAttributes` returns a new `AttributedString` with attributes removed. The one-argument form
empties the entire attribute overlay. The three-argument form clears every attribute over the
inclusive range `[start, endIndex]`, **splitting** any span that straddles the range so its flanks
outside the range survive (regardless of member — unlike `removeAttribute`, no structural match is
required)."#;

const EX: &str = r#"```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello world")
  MUT b AS AttributedString = astrings::addAttribute(a, 0, 10, astrings::bold())
  LET ranged AS AttributedString = astrings::clearAttributes(b, 2, 7)
  LET whole AS AttributedString = astrings::clearAttributes(b)
END SUB
```"#;

#[rustfmt::skip]
const BODY_WHOLE: &str =
r#"FUNC __astrings_clearAttributes(a AS AttributedString) AS AttributedString
  MUT empty AS List OF AttrSpan = []
  RETURN astrings::writeSpans(a, empty)
END FUNC"#;

#[rustfmt::skip]
const BODY_RANGE: &str =
r#"FUNC __astrings_clearAttributesRange(a AS AttributedString, start AS Integer, endIndex AS Integer) AS AttributedString
  LET n AS Integer = __astrings_validateRange(a, start, endIndex)
  LET spans AS List OF AttrSpan = astrings::readSpans(a)
  MUT out AS List OF AttrSpan = []
  FOR EACH s IN spans
    out = __astrings_splitSpan(out, s, start, endIndex)
  NEXT
  RETURN astrings::writeSpans(a, out)
END FUNC"#;

fn value_param() -> Parameter {
    Parameter {
        name: "value",
        desc: "The attributed string to clear.",
        aliases: &[],
        ty: ParameterType::named("AttributedString"),
        default: DefaultValue::None,
    }
}

fn integer_param(name: &'static str, desc: &'static str) -> Parameter {
    Parameter {
        name,
        desc,
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::None,
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "clearAttributes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![value_param()],
                return_type: ParameterType::named("AttributedString"),
                errors: vec![],
                body: Body::mfb(BODY_WHOLE, "__astrings_clearAttributes"),
            },
            Implementation {
                params: vec![
                    value_param(),
                    integer_param(
                        "start",
                        "(ranged form) The first scalar index of the range (0-based).",
                    ),
                    integer_param(
                        "endIndex",
                        "(ranged form) The last scalar index of the range (inclusive).",
                    ),
                ],
                return_type: ParameterType::named("AttributedString"),
                errors: vec![],
                body: Body::mfb(BODY_RANGE, "__astrings_clearAttributesRange"),
            },
        ],
    });
}

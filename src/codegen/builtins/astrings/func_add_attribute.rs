//! `astrings::addAttribute` — Tier-C mutation member (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_addAttribute` FUNC through the registry's `rewrite_target`.
//! The end-of-range parameter is `endIndex` (not `end`, a reserved keyword).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Record an attribute over an inclusive scalar range."#;

const DESC: &str = r#"`addAttribute` returns a new `AttributedString` with `attr` recorded over the **inclusive** scalar
range `[start, endIndex]` (length `endIndex − start + 1`; `start == endIndex` is a single scalar).
Spans are stored as-is and never merged; overlapping same-member spans resolve at read time by
higher-start-wins (see `getAttributes`). The end-of-range parameter is `endIndex` rather than `end`
because `end` is a reserved keyword."#;

const EX: &str = r#"```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello world")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::bold())
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_addAttribute(a AS AttributedString, start AS Integer, endIndex AS Integer, attr AS Attribute) AS AttributedString
  LET n AS Integer = __astrings_validateRange(a, start, endIndex)
  MUT spans AS List OF AttrSpan = astrings::readSpans(a)
  LET seq AS Integer = __astrings_nextSeq(spans)
  LET span AS AttrSpan = __astrings_encodeAttr(start, endIndex, seq, attr)
  spans = collections::append(spans, span)
  RETURN astrings::writeSpans(a, spans)
END FUNC"#;

fn ranged_attr_params() -> Vec<Parameter> {
    vec![
        Parameter {
            name: "value",
            desc: "The attributed string to add to.",
            aliases: &[],
            ty: ParameterType::named("AttributedString"),
            default: DefaultValue::None,
        },
        Parameter {
            name: "start",
            desc: "The first scalar index of the range (0-based).",
            aliases: &[],
            ty: ParameterType::Integer,
            default: DefaultValue::None,
        },
        Parameter {
            name: "endIndex",
            desc: "The last scalar index of the range (inclusive).",
            aliases: &[],
            ty: ParameterType::Integer,
            default: DefaultValue::None,
        },
        Parameter {
            name: "attr",
            desc: "The attribute to record (from a constructor such as `astrings::bold()`).",
            aliases: &[],
            ty: ParameterType::named("Attribute"),
            default: DefaultValue::None,
        },
    ]
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "addAttribute",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: ranged_attr_params(),
            return_type: ParameterType::named("AttributedString"),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_addAttribute"),
        }],
    });
}

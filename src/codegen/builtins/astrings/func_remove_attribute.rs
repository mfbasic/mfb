//! `astrings::removeAttribute` — Tier-C mutation member (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_removeAttribute` FUNC through the registry's `rewrite_target`.
//! The end-of-range parameter is `endIndex` (not `end`, a reserved keyword).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Remove a matching attribute over an inclusive scalar range, splitting straddlers."#;

const DESC: &str = r#"`removeAttribute` returns a new `AttributedString` with `attr` removed over the inclusive range
`[start, endIndex]`. A stored span is affected only when its attribute **structurally matches** `attr`
(same member and, for font/size, same value). A matching span that straddles the range is **split**:
its surviving left flank `[s.start, start−1]` and/or right flank `[endIndex+1, s.last]` are kept and
the overlap dropped. Because overlapping spans resolve by higher-start-wins, removing a covering
winner can reveal a lower-start loser at read time.

**An invalid range raises.** `start` must be zero or more and no greater than
`endIndex`, or the call raises `ErrInvalidArgument` (`invalid attribute range`);
and both ends must fall inside the visible text, or it raises
`ErrIndexOutOfRange` (`attribute range out of bounds`). Because the range is
inclusive, **empty text has no valid range at all** — even `0, 0` is out of
bounds on an empty `AttributedString`, so guard construction from text that
might be empty."#;

const EX: &str = r#"```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello world here")
  MUT b AS AttributedString = astrings::addAttribute(a, 5, 15, astrings::bold())
  b = astrings::removeAttribute(b, 8, 10, astrings::bold())
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_removeAttribute(a AS AttributedString, start AS Integer, endIndex AS Integer, attr AS Attribute) AS AttributedString
  LET n AS Integer = __astrings_validateRange(a, start, endIndex)
  LET spans AS List OF AttrSpan = astrings::readSpans(a)
  LET target AS AttrSpan = __astrings_encodeAttr(start, endIndex, 0, attr)
  MUT out AS List OF AttrSpan = []
  FOR EACH s IN spans
    IF __astrings_attrEquals(s, target) THEN
      out = __astrings_splitSpan(out, s, start, endIndex)
    ELSE
      out = collections::append(out, s)
    END IF
  NEXT
  RETURN astrings::writeSpans(a, out)
END FUNC"#;

fn ranged_attr_params() -> Vec<Parameter> {
    vec![
        Parameter {
            name: "value",
            desc: "The attributed string to remove from.",
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
            desc: "The attribute to remove (matched structurally).",
            aliases: &[],
            ty: ParameterType::named("Attribute"),
            default: DefaultValue::None,
        },
    ]
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "removeAttribute",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: ranged_attr_params(),
            return_type: ParameterType::named("AttributedString"),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_removeAttribute"),
        }],
    });
}

//! `astrings::getAttributes` — Tier-C query member (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_getAttributes` FUNC through the registry's `rewrite_target`.
//! Returns the winning attributes covering the scalar at `index`
//! (higher-start-wins resolution).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"The resolved attributes active at a scalar index."#;

const DESC: &str = r#"`getAttributes` returns the attributes in effect at scalar `index`: for each enum member with any
covering span, the covering span with the **highest start** wins (ties break to the later insertion).
The result carries at most one `Attribute` per member — flags are present when any covering span
carries them; font/font-size take the winning span's value. Attributes are never merged on write, so
this read-time resolution is where overlaps are decided."#;

const EX: &str = r#"```
IMPORT astrings
IMPORT io

SUB main()
  LET a AS AttributedString = astrings::fromString("hello world")
  MUT b AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::bold())
  FOR EACH attr IN astrings::getAttributes(b, 2)
    MATCH attr
      CASE AttrFlag(f)
        io::print("flag")
      CASE ELSE
    END MATCH
  NEXT
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_getAttributes(a AS AttributedString, index AS Integer) AS List OF Attribute
  LET n AS Integer = astrings::scalarLen(a)
  IF index < 0 OR index >= n THEN
    FAIL error(77050001, "attribute index out of bounds")
  END IF
  LET spans AS List OF AttrSpan = astrings::readSpans(a)
  MUT covering AS List OF AttrSpan = []
  FOR EACH s IN spans
    IF s.start <= index AND index <= s.last THEN
      covering = collections::append(covering, s)
    END IF
  NEXT
  MUT result AS List OF Attribute = []
  FOR EACH s IN covering
    IF __astrings_isWinner(s, covering) THEN
      result = collections::append(result, __astrings_decodeAttr(s))
    END IF
  NEXT
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getAttributes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The attributed string to query.",
                    aliases: &[],
                    ty: ParameterType::named("AttributedString"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "index",
                    desc: "The scalar index to resolve (0-based).",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::named("Attribute")),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_getAttributes"),
        }],
    });
}

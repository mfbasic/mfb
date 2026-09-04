//! `astrings::foreground` — `color::Color` `Attribute` constructor
//! (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_foreground` FUNC (which packs the colour into a `0xAARRGGBB`
//! numeric payload) through the registry's `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Construct a foreground-color `astrings::Attribute` from a `color::Color`."#;

const DESC: &str = r#"`foreground` returns an `astrings::Attribute` wrapping the `astrings::AttrNumber`
with `kind` `astrings::AttrTypeNumber.Foreground` and a `value` that packs `base`
into a single `0xAARRGGBB` Integer — alpha in the high byte, blue in the low one.
That is `color::toPacked`'s order, so `color::fromPacked` reads the attribute back
and the colour round-trips exactly, alpha included. Pass the attribute to
`astrings::addAttribute` to set the text foreground over a scalar range;
overlapping foreground spans resolve by higher-start-wins at read time.

**A program that names a `color::Color` must `IMPORT color`** as well as
`astrings` — imports are not transitive and a package cannot re-export another's
types.

When such an `AttributedString` is drawn with `term::drawText(x, y, value)` (both
`term` and `astrings` imported), the colour is emitted as a truecolor foreground.
**The terminal has no alpha and the bridge ignores it**: a half-transparent
foreground draws exactly the cells an opaque one draws. The alpha is preserved in
the attribute rather than dropped at construction, so a renderer that *can* model
it — a canvas surface, say — still gets the whole colour. Renderers that do not
model colour at all, such as `astrings::toMarkdown`, ignore the attribute
entirely."#;

const EX: &str = r#"```
IMPORT astrings
IMPORT color

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::foreground(color::rgb(255, 128, 0)))
END SUB
```

The alpha survives in the attribute even though a terminal cannot draw it:

```
IMPORT astrings
IMPORT color
IMPORT io

SUB main()
  LET a AS AttributedString = astrings::fromString("hi")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 1, astrings::foreground(color::rgba(255, 128, 0, 128)))
  FOR EACH at IN astrings::getAttributes(styled, 0)
    MATCH at
      CASE astrings::AttrNumber(nm)
        io::print(color::toHexAlpha(color::fromPacked(nm.value)))
      CASE ELSE
    END MATCH
  NEXT
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_foreground(base AS color::Color) AS Attribute
  RETURN AttrNumber[AttrTypeNumber.Foreground, color::toPacked(base)]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "foreground",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The foreground colour. Its alpha is carried in the attribute \
                       but ignored by the terminal bridge.",
                aliases: &[],
                ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named("Attribute"),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_foreground"),
        }],
    });
}

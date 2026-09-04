//! `astrings::background` — `color::Color` `Attribute` constructor
//! (`Body::Rewrite`).
//!
//! Backed by the injected source (the per-helper `helper_*.rs` bodies): a call rewrites to the
//! internal `__astrings_background` FUNC (which packs the colour into a `0xAARRGGBB`
//! numeric payload) through the registry's `rewrite_target`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Construct a background-color `astrings::Attribute` from a `color::Color`."#;

const DESC: &str = r#"`background` returns an `astrings::Attribute` wrapping the `astrings::AttrNumber`
with `kind` `astrings::AttrTypeNumber.Background` and a `value` that packs `base`
into a single `0xAARRGGBB` Integer — alpha in the high byte, blue in the low one.
That is `color::toPacked`'s order, so `color::fromPacked` reads the attribute back
and the colour round-trips exactly, alpha included. Pass the attribute to
`astrings::addAttribute` to set the text background over a scalar range;
overlapping background spans resolve by higher-start-wins at read time.

**A program that names a `color::Color` must `IMPORT color`** as well as
`astrings` — imports are not transitive and a package cannot re-export another's
types.

When such an `AttributedString` is drawn with `term::drawText(x, y, value)` (both
`term` and `astrings` imported), the colour is emitted as a truecolor background.
**The terminal has no alpha and the bridge ignores it**: a half-transparent
background draws exactly the cells an opaque one draws. The alpha is preserved in
the attribute rather than dropped at construction, so a renderer that *can* model
it still gets the whole colour. Renderers that do not model colour at all, such as
`astrings::toMarkdown`, ignore the attribute entirely."#;

const EX: &str = r#"```
IMPORT astrings
IMPORT color

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::background(color::rgb(0, 0, 128)))
END SUB
```

A foreground and a background over the same range:

```
IMPORT astrings
IMPORT color

SUB main()
  MUT a AS AttributedString = astrings::fromString("warning")
  a = astrings::addAttribute(a, 0, 6, astrings::foreground(color::rgb(255, 255, 0)))
  a = astrings::addAttribute(a, 0, 6, astrings::background(color::rgb(128, 0, 0)))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __astrings_background(base AS color::Color) AS Attribute
  RETURN AttrNumber[AttrTypeNumber.Background, color::toPacked(base)]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "background",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The background colour. Its alpha is carried in the attribute \
                       but ignored by the terminal bridge.",
                aliases: &[],
                ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named("Attribute"),
            errors: vec![],
            body: Body::mfb(BODY, "__astrings_background"),
        }],
    });
}

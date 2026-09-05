//! `color::nameOf` — the reverse CSS named-colour lookup.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"The CSS name of a colour, when it has one exactly."#;

const DESC: &str = r#"`nameOf` is the reverse of `color::fromName`: it returns the CSS Color Level 4
name of `base`, or raises `ErrNotFound` (`77050004`) if the colour is not in the
table.

**The match is exact, not nearest.** A colour one step off a named one has no
name, and asking for it is an error rather than an approximation. "Closest named
colour" is a different function, with a contestable metric and a much higher cost;
it is deliberately not this one.

`base` must be **fully opaque**. Every entry in the table has alpha `255`, so a
translucent colour has no name even when its red, green and blue match one
exactly — `nameOf` raises rather than quietly dropping the alpha.

Six colours have two CSS spellings, and `nameOf` returns the **alphabetically
first** of them — so `nameOf(fromName("grey"))` is `"gray"`, not `"grey"`. That
rule gives `gray`, `darkgray`, `lightgray`, `slategray`, `aqua` and `fuchsia`.
Both spellings still resolve through `color::fromName`; only the reverse
direction has to choose, and choosing by name rather than by table order means the
answer cannot change if the table is ever reordered.

This is what makes a colour round-trip through a config file readable: store
`nameOf` when it succeeds and `color::toHexAlpha` when it does not."#;

const EX: &str = r##"Name a colour that has one:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::nameOf(color::fromHex("#663399")))
END SUB
```

An unnamed colour, or a translucent one, raises `ErrNotFound`:

```
IMPORT color
IMPORT errorCode
IMPORT io

FUNC name(c AS color::Color) AS String
  RETURN color::nameOf(c)
  TRAP(err)
    RETURN "unnamed (" & toString(err.code = errorCode::ErrNotFound) & ")"
  END TRAP
END FUNC

SUB main()
  io::print(name(color::fromHex("#ff0000")))
  io::print(name(color::fromHex("#ff0001")))
  io::print(name(color::rgba(255, 0, 0, 128)))
END SUB
```"##;

/// A walk of the one shared table rather than a second reverse map: the reverse
/// direction is not in any hot path, and a second map would double the shipped data
/// — which is already the bulk of this package.
///
/// **The walk keeps the alphabetically smallest match rather than returning the
/// first one found**, and that is a determinism requirement, not a preference. Six
/// colours have two CSS spellings apiece, so for those the answer depends on which
/// key the walk meets first — and `collections::keys` order is documented as
/// *"implementation-defined but stable for a given unchanged map"*, i.e. insertion
/// order today but explicitly not a guarantee across versions. Returning the first
/// match would make `nameOf(fromName("grey"))` an artefact of table emission order,
/// and would flake this member's golden the day that order changed.
///
/// Taking the minimum is order-independent and picks the spelling callers expect in
/// every one of the six cases: `gray` < `grey`, `darkgray` < `darkgrey`,
/// `lightgray` < `lightgrey`, `slategray` < `slategrey`, `aqua` < `cyan`,
/// `fuchsia` < `magenta`.
///
/// The alpha gate is first and separate. Without it a translucent colour would fall
/// through to the walk and simply never match, which is the same *outcome* but
/// reports the wrong reason: the caller would be told the colour is unnamed when in
/// fact its hue is named and only its alpha disqualifies it.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_nameOf(base AS Color) AS String
  IF toInt(base.alpha) <> 255 THEN
    FAIL error(77050004, "a colour with alpha " & toString(base.alpha) & " has no CSS name")
  END IF
  LET packed AS Integer = color::toPacked(base)
  MUT best AS String = ""
  FOR EACH name IN collections::keys(__COLOR_NAMES)
    IF collections::get(__COLOR_NAMES, name) = packed THEN
      IF best = "" OR name < best THEN
        best = name
      END IF
    END IF
  NEXT
  IF best = "" THEN
    FAIL error(77050004, "no CSS name for " & color::toHex(base))
  END IF
  RETURN best
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "nameOf",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The colour to name. Must be fully opaque and match a CSS \
                       named colour exactly.",
                aliases: &[],
                ty: ParameterType::named(super::COLOR_TYPE),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec!["ErrNotFound"],
            body: Body::mfb(BODY, "__color_nameOf"),
        }],
    });
}

//! `color::toPacked` — a colour as one `0xAARRGGBB` integer.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Pack a colour into a single `0xAARRGGBB` integer."#;

const DESC: &str = r#"`toPacked` folds a colour's four channels into one `Integer`: alpha in the
highest byte, then red, then green, then blue in the lowest — `0xAARRGGBB`. The
result is always in `0`..`4294967295` (`0xFFFFFFFF`), never negative.

This is the form a colour takes when it has to travel through an API that carries
one number: a stored setting, a serialized attribute, a protocol field.
`color::fromPacked` is the exact inverse, so `fromPacked(toPacked(c))` is `c` for
every colour.

The alpha-high order is a deliberate choice and is the same one everywhere in
MFBASIC. Reading a packed colour that came from somewhere else, check which byte
its producer put alpha in before trusting it: `0xRRGGBBAA` is also in common use
and the two are indistinguishable by inspection.

To get the 24-bit `0xRRGGBB` form with alpha dropped, mask it off:
`bits::band(color::toPacked(c), 16777215)`."#;

const EX: &str = r#"The byte order, spelled out:

```
IMPORT color
IMPORT io

SUB main()
  ' red 0x12, green 0x34, blue 0x56, alpha 0x78 -> 0x78123456
  LET c AS color::Color = color::rgba(18, 52, 86, 120)
  io::print(toString(color::toPacked(c)))
END SUB
```

`color::fromPacked` is the exact inverse, so a colour survives the round trip:

```
IMPORT color
IMPORT io

SUB main()
  LET c AS color::Color = color::rgba(10, 20, 30, 40)
  LET back AS color::Color = color::fromPacked(color::toPacked(c))
  io::print(toString(back.red) & " " & toString(back.green) & " " & toString(back.blue) & " " & toString(back.alpha))
END SUB
```

Drop alpha to get the 24-bit `0xRRGGBB` form:

```
IMPORT bits
IMPORT color
IMPORT io

SUB main()
  LET c AS color::Color = color::rgba(51, 102, 204, 255)
  io::print(toString(bits::band(color::toPacked(c), 16777215)))
END SUB
```"#;

/// `bits::sl`/`bor` rather than `*`/`+` so the intent is a bit layout rather than
/// arithmetic that happens to land in the right place. Every channel is a `Byte`,
/// so no term can overflow into its neighbour and the `bor` chain needs no masks.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_toPacked(base AS Color) AS Integer
  LET high AS Integer = bits::bor(bits::sl(toInt(base.alpha), 24), bits::sl(toInt(base.red), 16))
  LET low AS Integer = bits::bor(bits::sl(toInt(base.green), 8), toInt(base.blue))
  RETURN bits::bor(high, low)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toPacked",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The colour to pack.",
                aliases: &[],
                ty: ParameterType::named(super::COLOR_TYPE),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::mfb(BODY, "__color_toPacked"),
        }],
    });
}

//! `color::fromPacked` — a colour from one `0xAARRGGBB` integer.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Unpack a `0xAARRGGBB` integer into a colour."#;

const DESC: &str = r#"`fromPacked` is the exact inverse of `color::toPacked`: it reads alpha from the
highest of the low four bytes, then red, then green, then blue —
`0xAARRGGBB` — and returns the colour they describe.

Only the low 32 bits of `value` are read. Anything above them is ignored, so a
value that arrived with high bits set, or a negative one, still yields a colour
rather than failing.

A 24-bit `0xRRGGBB` value — the form a CSS-style hex colour packs into — has a
zero top byte, and therefore unpacks to a **fully transparent** colour. That is
almost never what the caller meant. Add the alpha before unpacking
(`color::fromPacked(bits::bor(rgb24, 4278190080))`), or use `color::fromHex`,
which treats a missing alpha as opaque."#;

const EX: &str = r#"Unpack an opaque colour:

```
IMPORT color
IMPORT io

SUB main()
  ' 0xFF3366CC -> red 0x33, green 0x66, blue 0xCC, alpha 0xFF
  LET c AS color::Color = color::fromPacked(4281558732)
  io::print(toString(c.red) & " " & toString(c.green) & " " & toString(c.blue) & " " & toString(c.alpha))
END SUB
```

A 24-bit value has no alpha byte, so it unpacks fully transparent — add one
first:

```
IMPORT bits
IMPORT color
IMPORT io

SUB main()
  LET rgb24 AS Integer = 3368652        ' 0x3366CC
  LET bare AS color::Color = color::fromPacked(rgb24)
  LET opaque AS color::Color = color::fromPacked(bits::bor(rgb24, 4278190080))
  io::print(toString(bare.alpha) & " vs " & toString(opaque.alpha))
END SUB
```"#;

/// `bits::band` after every shift, including the alpha one: `bits::sr` is
/// zero-filling over 64 bits, so without the mask a `value` carrying anything
/// above bit 31 would leak into the alpha channel instead of being ignored.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_fromPacked(value AS Integer) AS Color
  LET red AS Byte = toByte(bits::band(bits::sr(value, 16), 255))
  LET green AS Byte = toByte(bits::band(bits::sr(value, 8), 255))
  LET blue AS Byte = toByte(bits::band(value, 255))
  LET alpha AS Byte = toByte(bits::band(bits::sr(value, 24), 255))
  RETURN Color[red, green, blue, alpha]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fromPacked",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The packed colour, `0xAARRGGBB`. Only the low 32 bits are read.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__color_fromPacked"),
        }],
    });
}

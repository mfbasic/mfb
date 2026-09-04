//! `color::toLinear` — the sRGB → linear-light transfer.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Convert one sRGB channel to its linear-light value, `0`..`65535`."#;

const DESC: &str = r#"`toLinear` maps an sRGB channel byte to the amount of light it represents, on a
scale of `0` (black) to `65535` (full). `color::fromLinear` is the inverse.

This pair is the seam every perceptual operation in `color` is built on, and it is
the same seam the canvas software rasteriser blends through, so a colour computed
here and a pixel drawn there cannot disagree.

**Why the conversion is not a multiplication.** An sRGB channel is *encoded*, not
proportional to light: `128` is not half as much light as `255`, it is about 22%.
Blending, brightening or averaging the encoded bytes directly produces the
familiar too-dark midpoint. Convert to linear, do the arithmetic, convert back.

The mapping is a fixed 256-entry table rather than a computed power function.
That is deliberate: the software rasteriser is the oracle the GPU backends are
compared against, so it must produce identical bytes on every target, and a libm
transcendental does not. The table's endpoints are exact — `toLinear(0)` is `0`
and `toLinear(255)` is `65535` — which is what makes an opaque blend land on its
source exactly rather than a step below.

Round-tripping is exact in this direction: `fromLinear(toLinear(c))` is `c` for
all 256 channel values. The other direction is not, and cannot be — there are
65536 linear values and only 256 channels."#;

const EX: &str = r#"The encoding is not proportional — the midpoint is far below half:

```
IMPORT color
IMPORT io

SUB main()
  io::print(toString(color::toLinear(toByte(0))))
  io::print(toString(color::toLinear(toByte(128))))
  io::print(toString(color::toLinear(toByte(255))))
END SUB
```

Every channel survives a round trip through linear light:

```
IMPORT color
IMPORT io

SUB main()
  io::print(toString(color::fromLinear(color::toLinear(toByte(200)))))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_toLinear(channel AS Byte) AS Integer
  RETURN collections::getOr(__COLOR_SRGB, toInt(channel), 0)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toLinear",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "channel",
                desc: "The sRGB channel to convert, `0`..`255`.",
                aliases: &[],
                ty: ParameterType::Byte,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::mfb(BODY, "__color_toLinear"),
        }],
    });
}

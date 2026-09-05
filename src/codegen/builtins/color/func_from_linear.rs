//! `color::fromLinear` — the linear-light → sRGB transfer.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Convert a linear-light value, `0`..`65535`, back to an sRGB channel."#;

const DESC: &str = r#"`fromLinear` is the inverse of `color::toLinear`: it returns the sRGB channel
byte whose linear value is nearest to `value`. Together they are the seam every
perceptual operation in `color` is built on, and the one the canvas software
rasteriser blends through.

`value` is clamped by the search rather than rejected: anything at or below `0`
yields `0` and anything at or past `65535` yields `255`, so an intermediate that
overshot by a rounding step does not need guarding before the call.

The answer is found by binary search over the same 256-entry table `toLinear`
reads — eight comparisons, and exactly as deterministic as a lookup. A reverse
table would need 65536 entries to say the same thing.

The mapping is **not** one-to-one in this direction, and cannot be: there are
65536 linear values and 256 channels. `fromLinear(toLinear(c))` is `c` for every
channel, but `toLinear(fromLinear(v))` is only `v` when `v` is one of the 256
representable linear values."#;

const EX: &str = r#"The endpoints are exact, and the round trip through a channel is too:

```
IMPORT color
IMPORT io

SUB main()
  io::print(toString(color::fromLinear(0)))
  io::print(toString(color::fromLinear(65535)))
  io::print(toString(color::fromLinear(color::toLinear(toByte(200)))))
END SUB
```

Average two colours correctly — in linear light, not in encoded bytes:

```
IMPORT color
IMPORT io

SUB main()
  LET a AS Byte = toByte(0)
  LET b AS Byte = toByte(255)
  LET midLinear AS Integer = (color::toLinear(a) + color::toLinear(b)) / 2
  io::print("linear midpoint " & toString(color::fromLinear(midLinear)))
  io::print("byte midpoint " & toString((toInt(a) + toInt(b)) / 2))
END SUB
```"#;

/// The binary search moved verbatim from canvas's `__canvas_linearToSrgb`, down to
/// the `getOr` defaults (`0` low, `65535` high) and the midpoint comparison. It is
/// the same expression, so the move cannot shift a rendered pixel by a rounding
/// step.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_fromLinear(value AS Integer) AS Byte
  MUT lo AS Integer = 0
  MUT hi AS Integer = 255
  WHILE lo < hi
    LET mid AS Integer = (lo + hi) / 2
    LET midLow AS Integer = collections::getOr(__COLOR_SRGB, mid, 0)
    LET midHigh AS Integer = collections::getOr(__COLOR_SRGB, mid + 1, 65535)
    IF value > (midLow + midHigh) / 2 THEN
      lo = mid + 1
    ELSE
      hi = mid
    END IF
  END WHILE
  RETURN toByte(lo)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fromLinear",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The linear-light value, `0`..`65535`. Values outside that \
                       range saturate to `0` or `255` rather than raising.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Byte,
            errors: vec![],
            body: Body::mfb(BODY, "__color_fromLinear"),
        }],
    });
}

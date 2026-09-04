//! The HSL conversion core — shared by `toHsl`, `hsl`/`hsla` and the three
//! manipulators.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// **HSL is computed on the sRGB channels, not on linear light**, and that is a
/// deliberate asymmetry with `brighten`/`darken`/`mix`. CSS `hsl()` and every
/// design tool mean the encoded channels; a `toHsl`/`hsl` round trip that did not
/// agree with the hex a designer pasted in would be useless. Both `toHsl` and
/// `brighten` say so on their pages, so a reader cannot form the wrong mental model
/// from either one alone.
///
/// **No transcendentals.** The whole conversion is `+ - * /`, comparisons, and
/// `math::abs`/`floor`/`min`/`max` — no trig, despite hue being an angle. That is
/// not stylistic: canvas's software rasteriser is the oracle its GPU backends are
/// compared against, and since plan-122-B canvas calls into `color`, so the rule
/// covers this file too.
///
/// `__color_hueSector` is the sector permutation shared by the forward conversion;
/// writing it once keeps the six arms from drifting apart, which is the classic way
/// an HSL implementation ends up correct for four hues and wrong for two.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_channelFraction(channel AS Byte) AS Float
  RETURN toFloat(toInt(channel)) / 255.0
END FUNC

FUNC __color_fractionChannel(value AS Float) AS Byte
  RETURN __color_clampByte(toInt(value * 255.0 + 0.5))
END FUNC

FUNC __color_wrapHue(hue AS Float) AS Float
  LET turns AS Float = hue / 360.0
  LET wrapped AS Float = hue - toFloat(math::floor(turns)) * 360.0
  IF wrapped < 0.0 THEN
    RETURN wrapped + 360.0
  END IF
  IF wrapped >= 360.0 THEN
    RETURN 0.0
  END IF
  RETURN wrapped
END FUNC

FUNC __color_hueSector(sector AS Integer, c AS Float, x AS Float, channel AS Integer) AS Float
  IF sector = 0 THEN
    IF channel = 0 THEN
      RETURN c
    END IF
    IF channel = 1 THEN
      RETURN x
    END IF
    RETURN 0.0
  END IF
  IF sector = 1 THEN
    IF channel = 0 THEN
      RETURN x
    END IF
    IF channel = 1 THEN
      RETURN c
    END IF
    RETURN 0.0
  END IF
  IF sector = 2 THEN
    IF channel = 0 THEN
      RETURN 0.0
    END IF
    IF channel = 1 THEN
      RETURN c
    END IF
    RETURN x
  END IF
  IF sector = 3 THEN
    IF channel = 0 THEN
      RETURN 0.0
    END IF
    IF channel = 1 THEN
      RETURN x
    END IF
    RETURN c
  END IF
  IF sector = 4 THEN
    IF channel = 0 THEN
      RETURN x
    END IF
    IF channel = 1 THEN
      RETURN 0.0
    END IF
    RETURN c
  END IF
  IF channel = 0 THEN
    RETURN c
  END IF
  IF channel = 1 THEN
    RETURN 0.0
  END IF
  RETURN x
END FUNC

FUNC __color_hslToColor(hue AS Float, saturation AS Float, lightness AS Float, alpha AS Integer) AS Color
  LET h AS Float = __color_wrapHue(hue)
  LET s AS Float = __color_clampFraction(saturation)
  LET l AS Float = __color_clampFraction(lightness)
  LET c AS Float = (1.0 - math::abs(2.0 * l - 1.0)) * s
  LET sixth AS Float = h / 60.0
  LET sector AS Integer = math::floor(sixth)
  LET within AS Float = sixth - toFloat(math::floor(sixth / 2.0)) * 2.0
  LET x AS Float = c * (1.0 - math::abs(within - 1.0))
  LET m AS Float = l - c / 2.0
  LET red AS Byte = __color_fractionChannel(__color_hueSector(sector, c, x, 0) + m)
  LET green AS Byte = __color_fractionChannel(__color_hueSector(sector, c, x, 1) + m)
  LET blue AS Byte = __color_fractionChannel(__color_hueSector(sector, c, x, 2) + m)
  RETURN Color[red, green, blue, __color_clampByte(alpha)]
END FUNC

FUNC __color_colorToHsl(base AS Color) AS Hsl
  LET r AS Float = __color_channelFraction(base.red)
  LET g AS Float = __color_channelFraction(base.green)
  LET b AS Float = __color_channelFraction(base.blue)
  LET high AS Float = math::max(r, math::max(g, b))
  LET low AS Float = math::min(r, math::min(g, b))
  LET delta AS Float = high - low
  LET lightness AS Float = (high + low) / 2.0
  IF delta <= 0.0 THEN
    RETURN Hsl[0.0, 0.0, lightness]
  END IF
  LET saturation AS Float = delta / (1.0 - math::abs(2.0 * lightness - 1.0))
  MUT hue AS Float = 0.0
  IF high = r THEN
    hue = 60.0 * ((g - b) / delta)
  END IF
  IF high <> r AND high = g THEN
    hue = 60.0 * ((b - r) / delta + 2.0)
  END IF
  IF high <> r AND high <> g THEN
    hue = 60.0 * ((r - g) / delta + 4.0)
  END IF
  RETURN Hsl[__color_wrapHue(hue), __color_clampFraction(saturation), lightness]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("color_hsl", BODY));
}

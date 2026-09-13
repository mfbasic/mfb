### 1. HSL conversion drops transparency
UNIT:      man-page:color/types  
CLAIM:     "`color::toHsl` returns one, and `color::hsl` builds a colour back from the same three values."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/color/func_to_hsl.rs:__color_toHsl` returns only `Hsl`; `func_hsl.rs:__color_hsl` always calls `__color_hslToColor(..., 255)`. Probe compiled and ran with the specified binary: `rgba(255, 0, 0, 0)` printed `#ff000000`, while rebuilding its `toHsl` fields with `hsl` printed `#ff0000ff`.  
SUGGESTED: `color::toHsl returns the RGB-derived HSL fields only; it does not retain alpha. color::hsl builds an opaque colour, so use color::hsla with the original alpha when transparency must be preserved.`

### 2. Hsl field ranges are ambiguous and not enforced
UNIT:      man-page:color/types  
CLAIM:     "The hue in degrees around the colour wheel, 0.0..360.0."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/color/helper_hsl.rs:__color_wrapHue` normalizes values returned by `toHsl` to `0.0 <= hue < 360.0`; `360.0` becomes `0.0`. However, `color::Hsl` is a public record with unrestricted `Float` fields. The compiled probe printed a directly constructed `color::Hsl[720.0, -1.0, 2.0]` unchanged as `720.00,-1.00,2.00`.  
SUGGESTED: `When returned by color::toHsl, hue is at least 0.0 and less than 360.0; a colour with no saturation reports 0.0. Hsl literals are not range-checked; color::hsl wraps hue and clamps saturation and lightness.`
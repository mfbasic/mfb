### 1. Hue upper bound is unstated
UNIT:      man-page:color/toHsl  
CLAIM:     "`toHsl` returns a `color::Hsl` describing `base`: `hue` in degrees `0.0`..`360.0`, `saturation` and `lightness` in `0.0`..`1.0`."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/color/helper_hsl.rs:__color_colorToHsl` returns `__color_wrapHue(hue)`; `__color_wrapHue` returns `0.0` when the wrapped value is `>= 360.0`, so the returned hue range is `0.0 <= hue < 360.0`. Probe `probe/src/main.mfb` printed primary hues `0.00`, `120.00`, `240.00`, and `300.00`.  
SUGGESTED: "`toHsl` returns a `color::Hsl` describing `base`: `hue` in degrees from `0.0` inclusive to `360.0` exclusive, and `saturation` and `lightness` in `0.0`..`1.0`."

### 2. “Exact” grey round trip loses alpha
UNIT:      man-page:color/toHsl  
CLAIM:     "The round trip is still exact: `hsl` ignores hue entirely when saturation is `0.0`, so `hsl(toHsl(grey))` returns the grey."  
VERDICT:   wrong  
EVIDENCE:  `src/codegen/builtins/color/func_to_hsl.rs:BODY` returns only `Hsl`; `src/codegen/builtins/color/func_hsl.rs:BODY` rebuilds with alpha fixed at `255`. Probe `probe/src/main.mfb` printed `alpha #80808040 -> #808080ff` for `rgba(128, 128, 128, 64)`.  
SUGGESTED: "For an opaque grey, the colour-channel round trip is exact: `hsl` ignores hue when saturation is `0.0`. `toHsl` does not retain alpha, so use `hsla` with the original alpha when rebuilding a transparent colour."